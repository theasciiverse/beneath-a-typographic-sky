//! The terminal host: shows the engine's frames as text cells, turns
//! terminal mouse and keys into game input, and draws the menus.
//!
//! Run with `RUST_LOG=debug typosky 2>typosky.log` to log every input
//! event, mode switch and a state snapshot each second.

use std::collections::VecDeque;
use std::io::{self, stdout};
use std::time::{Duration, Instant};

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::DefaultTerminal;

use super::raster::{self, Cell, Image, Mode, Rgb};
use crate::engine::host::{Action, Frame, Host, HostEvent, MenuItem, TextOverlay};

const GAME_W: usize = 320;
const GAME_H: usize = 200;

/// Where the game picture sits in the terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    mode: Mode,
    x: u16,
    y: u16,
    cols: u16,
    rows: u16,
}

pub struct TermHost {
    terminal: DefaultTerminal,
    start: Instant,
    queue: VecDeque<HostEvent>,
    held: bool,
    /// None picks the mode by terminal size
    forced_mode: Option<Mode>,
    layout: Layout,
    /// what was last drawn, so identical frames are skipped
    last_key: u64,
    last_draw: Instant,
    /// last frame as cells, kept for drawing menus over it
    cells: Vec<Cell>,
    texts: Vec<TextOverlay>,
    last_snapshot: Instant,
    pub audio: bool,
    /// the game pointer as a glyph: mouse cell and character
    pointer: Option<(u16, u16, char)>,
    /// set by SIGTERM, SIGHUP or SIGQUIT: quit (and so autosave) cleanly
    signalled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl TermHost {
    pub fn new() -> io::Result<TermHost> {
        let terminal = ratatui::init();
        execute!(stdout(), EnableMouseCapture)?;
        let mut h = TermHost {
            terminal,
            start: Instant::now(),
            queue: VecDeque::new(),
            held: false,
            forced_mode: None,
            layout: Layout { mode: Mode::Braille, x: 0, y: 0, cols: 160, rows: 50 },
            last_key: 0,
            last_draw: Instant::now(),
            cells: Vec::new(),
            texts: Vec::new(),
            last_snapshot: Instant::now(),
            audio: true,
            pointer: None,
            signalled: Default::default(),
        };
        // The first signal asks for a clean quit (with its autosave); a
        // second one ends the game at once, so it can never hang a kill.
        let mut sigs = signal_hook::consts::TERM_SIGNALS.to_vec();
        #[cfg(unix)]
        sigs.push(signal_hook::consts::SIGHUP);
        for sig in sigs {
            let _ = signal_hook::flag::register_conditional_shutdown(sig, 1, h.signalled.clone());
            let _ = signal_hook::flag::register(sig, h.signalled.clone());
        }
        h.relayout();
        Ok(h)
    }

    fn relayout(&mut self) {
        let size = self.terminal.size().unwrap_or_default();
        let (tw, th) = (size.width, size.height);
        // native sizes: half blocks 320x100 cells, Braille 160x50
        let mode = self.forced_mode.unwrap_or(if tw >= 320 && th >= 100 { Mode::HalfBlock } else { Mode::Braille });
        let (cw, ch) = mode.dots();
        let (nat_c, nat_r) = ((GAME_W / cw) as u16, (GAME_H / ch) as u16);
        let (cols, rows) = if tw >= nat_c && th >= nat_r {
            (nat_c, nat_r)
        } else {
            // shrink, keeping the picture's shape
            let by_w = (tw, (tw as u32 * nat_r as u32 / nat_c as u32) as u16);
            if by_w.1 <= th {
                by_w
            } else {
                ((th as u32 * nat_c as u32 / nat_r as u32) as u16, th)
            }
        };
        let cols = cols.max(1);
        let rows = rows.max(1);
        self.layout = Layout { mode, x: (tw.saturating_sub(cols)) / 2, y: (th.saturating_sub(rows)) / 2, cols, rows };
        self.last_key = 0;
        log::debug!("layout {:?} in a {tw}x{th} terminal", self.layout);
    }

    /// Terminal cell to game pixel (the middle of the cell's area).
    fn to_game(&self, col: u16, row: u16) -> (u16, u16) {
        let l = self.layout;
        let cx = col.saturating_sub(l.x).min(l.cols - 1) as u32;
        let cy = row.saturating_sub(l.y).min(l.rows - 1) as u32;
        let mut x = ((cx * 2 + 1) * GAME_W as u32 / (l.cols as u32 * 2)) as u16;
        let mut y = ((cy * 2 + 1) * GAME_H as u32 / (l.rows as u32 * 2)) as u16;
        // A cell on the picture's edge stands for the edge itself, as a
        // mouse pushed against the screen edge would: the inventory opens
        // only on pixel rows 0 and 1, which no cell middle reaches.
        if cx == 0 {
            x = 0;
        } else if cx == l.cols as u32 - 1 {
            x = GAME_W as u16 - 1;
        }
        if cy == 0 {
            y = 0;
        } else if cy == l.rows as u32 - 1 {
            y = GAME_H as u16 - 1;
        }
        (x.min(GAME_W as u16 - 1), y.min(GAME_H as u16 - 1))
    }

    fn handle(&mut self, ev: Event) {
        match ev {
            Event::Key(k) if k.kind != KeyEventKind::Release => {
                log::debug!("key {:?} {:?}", k.code, k.modifiers);
                if let Some(e) = self.key(k) {
                    self.queue.push_back(e);
                }
            }
            Event::Mouse(m) => {
                let (x, y) = self.to_game(m.column, m.row);
                match m.kind {
                    MouseEventKind::Moved | MouseEventKind::Drag(_) => self.queue.push_back(HostEvent::MouseMove(x, y)),
                    MouseEventKind::Down(b) => {
                        log::debug!("mouse {b:?} down at cell {},{} = game {x},{y}", m.column, m.row);
                        self.held = true;
                        let button = if b == MouseButton::Right { 1 } else { 2 };
                        self.queue.push_back(HostEvent::Button { x, y, button });
                    }
                    MouseEventKind::Up(_) => {
                        self.held = false;
                        self.queue.push_back(HostEvent::ButtonUp);
                    }
                    _ => {}
                }
            }
            Event::Resize(..) => self.relayout(),
            _ => {}
        }
    }

    fn key(&mut self, k: KeyEvent) -> Option<HostEvent> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        Some(match k.code {
            KeyCode::Char('c') if ctrl => HostEvent::Quit,
            KeyCode::Char('f') if ctrl => HostEvent::Action(Action::ToggleFastMode),
            KeyCode::Char('g') if ctrl => HostEvent::Action(Action::ToggleReallyFastMode),
            KeyCode::F(5) | KeyCode::Char('q') | KeyCode::Char('Q') => HostEvent::Action(Action::OpenControlPanel),
            KeyCode::Esc => HostEvent::Action(Action::Skip),
            KeyCode::Char('.') => HostEvent::Action(Action::SkipLine),
            KeyCode::Char('p') | KeyCode::Char('P') => HostEvent::Action(Action::Pause),
            KeyCode::F(2) => {
                self.forced_mode = Some(match self.layout.mode {
                    Mode::Braille => Mode::HalfBlock,
                    Mode::HalfBlock => Mode::Quadrant,
                    Mode::Quadrant => Mode::Braille,
                });
                self.relayout();
                log::debug!("render mode {:?}", self.layout.mode);
                return None;
            }
            _ => HostEvent::Key,
        })
    }

    fn pump(&mut self, wait: Duration) {
        if self.signalled.load(std::sync::atomic::Ordering::Relaxed) {
            log::info!("signal received, quitting");
            self.queue.push_back(HostEvent::Quit);
        }
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match event::poll(left).and_then(|ready| if ready { event::read().map(Some) } else { Ok(None) }) {
                Ok(Some(ev)) => self.handle(ev),
                Ok(None) => break,
                Err(e) => {
                    // the terminal is gone (closed window, killed tmux): quit
                    // instead of running on with nobody to see it
                    log::info!("terminal input failed, quitting: {e}");
                    self.queue.push_back(HostEvent::Quit);
                    std::thread::sleep(left.min(Duration::from_millis(50)));
                    break;
                }
            }
            if Instant::now() >= deadline {
                break;
            }
        }
    }

    fn draw(&mut self) {
        let l = self.layout;
        let cells = &self.cells;
        let texts = &self.texts;
        let pointer = self.pointer;
        let _ = self.terminal.draw(|f| {
            let buf = f.buffer_mut();
            draw_cells(buf, l, cells);
            draw_texts(buf, l, texts);
            if let Some((col, row, ch)) = pointer {
                // just above and right of the terminal arrow's tip, which
                // covers the cell it points at and the one below
                let (x, y) = (col + 1, row.saturating_sub(1).max(l.y));
                let area = buf.area;
                if x < area.right() && y < area.bottom() {
                    let cell = &mut buf[(x, y)];
                    let under = match cell.bg {
                        Color::Rgb(r, g, b) => Color::Rgb(r / 4, g / 4, b / 4),
                        _ => Color::Black,
                    };
                    cell.set_char(ch)
                        .set_fg(Color::Rgb(255, 214, 90))
                        .set_bg(under)
                        .set_style(Style::default().add_modifier(Modifier::BOLD));
                }
            }
        });
    }

    /// A centred card of text, shown until a key or click, or `ms`.
    pub fn splash(&mut self, lines: &[&str], ms: u64) {
        let _ = self.terminal.draw(|f| {
            let area = f.area();
            let buf = f.buffer_mut();
            let style = Style::default().fg(Color::Rgb(150, 210, 160)).bg(Color::Black);
            for y in area.top()..area.bottom() {
                for x in area.left()..area.right() {
                    buf[(x, y)].set_char(' ').set_style(style);
                }
            }
            let top = area.height.saturating_sub(lines.len() as u16) / 2;
            for (i, l) in lines.iter().enumerate() {
                let w = l.chars().count() as u16;
                let st = if i == 0 { style.add_modifier(Modifier::BOLD) } else { style };
                buf.set_stringn(area.width.saturating_sub(w) / 2, top + i as u16, l, area.width as usize, st);
            }
        });
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            if let Ok(true) = event::poll(end.saturating_duration_since(Instant::now())) {
                match event::read() {
                    Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => break,
                    Ok(Event::Mouse(m)) if matches!(m.kind, MouseEventKind::Down(_)) => break,
                    _ => {}
                }
            }
        }
    }

    /// Draws a box with `lines` over the last frame; `sel` is highlighted.
    fn draw_menu(&mut self, title: &str, items: &[MenuItem], sel: usize) {
        let l = self.layout;
        let cells = &self.cells;
        let texts = &self.texts;
        let _ = self.terminal.draw(|f| {
            let area = f.area();
            let buf = f.buffer_mut();
            draw_cells(buf, l, cells);
            draw_texts(buf, l, texts);
            let w = items.iter().map(|i| i.label.chars().count()).max().unwrap_or(0).max(title.chars().count()) as u16 + 6;
            let h = items.len() as u16 + 4;
            let r = Rect::new(area.width.saturating_sub(w) / 2, area.height.saturating_sub(h) / 2, w.min(area.width), h.min(area.height));
            let frame = Style::default().fg(Color::Rgb(120, 200, 140)).bg(Color::Rgb(8, 14, 10));
            for y in r.top()..r.bottom() {
                for x in r.left()..r.right() {
                    buf[(x, y)].set_char(' ').set_style(frame);
                }
            }
            let block = ratatui::widgets::Block::bordered().title(format!(" {title} ")).style(frame);
            ratatui::widgets::Widget::render(block, r, buf);
            for (i, it) in items.iter().enumerate() {
                let y = r.y + 2 + i as u16;
                let mut st = frame;
                if !it.enabled {
                    st = st.fg(Color::Rgb(70, 90, 75));
                }
                if i == sel {
                    st = st.add_modifier(Modifier::REVERSED);
                }
                buf.set_stringn(r.x + 3, y, &it.label, (w - 6) as usize, st);
            }
        });
    }
}

impl Drop for TermHost {
    fn drop(&mut self) {
        let _ = execute!(stdout(), DisableMouseCapture);
        ratatui::restore();
    }
}

fn rgb(c: Rgb) -> Color {
    Color::Rgb(c[0], c[1], c[2])
}

fn draw_cells(buf: &mut Buffer, l: Layout, cells: &[Cell]) {
    let area = buf.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].set_char(' ').set_bg(Color::Black).set_fg(Color::Black);
        }
    }
    for (i, c) in cells.iter().enumerate() {
        let x = l.x + (i % l.cols as usize) as u16;
        let y = l.y + (i / l.cols as usize) as u16;
        if x < area.right() && y < area.bottom() {
            buf[(x, y)].set_char(c.ch).set_fg(rgb(c.fg)).set_bg(rgb(c.bg));
        }
    }
}

/// Sets the game's text sprites as real characters, centred in the area
/// the pixel text would have covered.
fn draw_texts(buf: &mut Buffer, l: Layout, texts: &[TextOverlay]) {
    let area = buf.area;
    let sx = l.cols as f32 / GAME_W as f32;
    let sy = l.rows as f32 / GAME_H as f32;
    for t in texts {
        if let Some(h) = t.boxed {
            draw_button(buf, l, t, h);
            continue;
        }
        let color = t.color_rgb;
        let lh = (t.line_height as f32 * sy).max(1.0);
        for (i, line) in t.lines.iter().enumerate() {
            let n = line.chars().count() as i32;
            let left = t.x as f32 * sx;
            let box_w = t.width as f32 * sx;
            let x0 = if t.centered {
                (left + (box_w - n as f32) / 2.0).round() as i32
            } else {
                left.round() as i32
            };
            let y = (t.y as f32 * sy + i as f32 * lh + (lh - 1.0) / 2.0).round() as i32 + l.y as i32;
            if y < area.top() as i32 || y >= area.bottom() as i32 {
                continue;
            }
            // keep the line inside the picture
            let x0 = x0.clamp(0, (l.cols as i32 - n).max(0)) + l.x as i32;
            for (j, ch) in line.chars().enumerate() {
                let x = x0 + j as i32;
                if x < area.left() as i32 || x >= area.right() as i32 {
                    continue;
                }
                let cell = &mut buf[(x as u16, y as u16)];
                let under = match cell.bg {
                    Color::Rgb(r, g, b) => [r / 5, g / 5, b / 5],
                    _ => [0, 0, 0],
                };
                cell.set_char(ch).set_fg(rgb(color)).set_bg(rgb(under)).set_style(Style::default().add_modifier(Modifier::BOLD));
            }
        }
    }
}

/// A pixel button as a framed box with its label in type.
fn draw_button(buf: &mut Buffer, l: Layout, t: &TextOverlay, h: u16) {
    let area = buf.area;
    let sx = l.cols as f32 / GAME_W as f32;
    let sy = l.rows as f32 / GAME_H as f32;
    let x0 = (t.x as f32 * sx).round() as i32 + l.x as i32;
    let y0 = (t.y as f32 * sy).round() as i32 + l.y as i32;
    let x1 = ((t.x + t.width as i32) as f32 * sx).round() as i32 + l.x as i32 - 1;
    let y1 = ((t.y + h as i32) as f32 * sy).round() as i32 + l.y as i32 - 1;
    let fg = rgb(t.color_rgb);
    let bg = Color::Rgb(16, 18, 22);
    let inside = |x: i32, y: i32| x >= area.left() as i32 && x < area.right() as i32 && y >= area.top() as i32 && y < area.bottom() as i32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            if !inside(x, y) {
                continue;
            }
            let ch = match (x == x0, x == x1, y == y0, y == y1) {
                (true, _, true, _) => '┌',
                (_, true, true, _) => '┐',
                (true, _, _, true) => '└',
                (_, true, _, true) => '┘',
                (_, _, true, _) | (_, _, _, true) => '─',
                (true, _, _, _) | (_, true, _, _) => '│',
                _ => ' ',
            };
            buf[(x as u16, y as u16)].set_char(ch).set_fg(fg).set_bg(bg).set_style(Style::default());
        }
    }
    let label = t.lines.first().map(String::as_str).unwrap_or("");
    if label == "◀" || label == "▶" {
        draw_arrow(buf, (x0 + 1, y0 + 1, x1 - 1, y1 - 1), label == "◀", fg, bg);
        return;
    }
    let n = label.chars().count() as i32;
    let lx = x0 + ((x1 - x0 + 1) - n) / 2;
    let ly = (y0 + y1) / 2;
    for (j, ch) in label.chars().enumerate() {
        let x = lx + j as i32;
        if inside(x, ly) && x > x0 && x < x1 {
            buf[(x as u16, ly as u16)].set_char(ch).set_fg(fg).set_bg(bg).set_style(Style::default().add_modifier(Modifier::BOLD));
        }
    }
}

/// A big triangle in half blocks filling the box (x0, y0, x1, y1),
/// pointing left or right. Half blocks give two square-ish dots per cell.
fn draw_arrow(buf: &mut Buffer, (x0, y0, x1, y1): (i32, i32, i32, i32), left: bool, fg: Color, bg: Color) {
    let area = buf.area;
    let (w, h) = (x1 - x0 + 1, (y1 - y0 + 1) * 2); // in half-block dots
    if w < 2 || h < 2 {
        return;
    }
    // leave a margin; half-block dots are about square, so an
    // equilateral triangle is about 0.87 times as wide as it is tall
    let th = (h - 2).max(2);
    let tw = ((th as f32 * 0.87).round() as i32).min(w - 2).max(1);
    let (ox, oy) = (x0 + (w - tw) / 2, (h - th) / 2);
    let on = |dx: i32, dy: i32| -> bool {
        // dy from 0..th, dx from 0..tw; the apex is at the pointing side
        let half = th as f32 / 2.0;
        let reach = tw as f32 * (1.0 - ((dy as f32 + 0.5) - half).abs() / half);
        let d = if left { tw - 1 - dx } else { dx };
        (d as f32) < reach
    };
    for cy in 0..(y1 - y0 + 1) {
        for cx in 0..w {
            let (x, y) = (x0 + cx, y0 + cy);
            if x < area.left() as i32 || x >= area.right() as i32 || y < area.top() as i32 || y >= area.bottom() as i32 {
                continue;
            }
            let dot = |half: i32| {
                let (dx, dy) = (cx - (ox - x0), cy * 2 + half - oy);
                dx >= 0 && dx < tw && dy >= 0 && dy < th && on(dx, dy)
            };
            let ch = match (dot(0), dot(1)) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                _ => ' ',
            };
            buf[(x as u16, y as u16)].set_char(ch).set_fg(fg).set_bg(bg).set_style(Style::default());
        }
    }
}

/// The glyph for each kind of game pointer (sky's MOUSE_* frames).
fn pointer_glyph(c: &crate::engine::host::Cursor) -> Option<char> {
    if !c.visible {
        return None;
    }
    match c.kind {
        0 => Some('◆'), // an object in hand
        2 => Some('◷'), // disk: wait
        3 => Some('↓'),
        4 => Some('→'),
        5 => Some('←'),
        7 => Some('+'), // over something
        8 => Some('↑'),
        _ => None, // 1 normal, 6 blank
    }
}

fn frame_key(frame: &Frame, l: Layout) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    frame.pixels.hash(&mut h);
    frame.palette.hash(&mut h);
    frame.texts.hash(&mut h);
    frame.cursor.visible.hash(&mut h);
    frame.cursor.kind.hash(&mut h);
    frame.cursor.pixels.hash(&mut h);
    (frame.mouse_x, frame.mouse_y).hash(&mut h);
    (l.cols, l.rows, l.x, l.y, l.mode as u8).hash(&mut h);
    h.finish()
}

impl Host for TermHost {
    fn millis(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn sleep(&mut self, ms: u32) {
        self.pump(Duration::from_millis(ms as u64));
    }

    fn poll(&mut self) -> Vec<HostEvent> {
        self.pump(Duration::ZERO);
        self.queue.drain(..).collect()
    }

    fn present(&mut self, frame: &Frame) {
        let key = frame_key(frame, self.layout);
        if key == self.last_key || self.last_draw.elapsed() < Duration::from_millis(15) {
            return;
        }
        self.last_key = key;
        self.last_draw = Instant::now();

        // palette indices to colours, with the pointer on top
        let pal = frame.palette;
        let px: Vec<Rgb> = frame
            .pixels
            .iter()
            .map(|&p| {
                let i = p as usize * 3;
                [pal[i], pal[i + 1], pal[i + 2]]
            })
            .collect();
        // The game's pointer as one glyph in type (David, 2026-10-17);
        // none over plain floor, where the terminal's own arrow serves.
        let l = self.layout;
        self.pointer = pointer_glyph(frame.cursor).map(|ch| {
            let col = l.x as u32 + frame.mouse_x as u32 * l.cols as u32 / GAME_W as u32;
            let row = l.y as u32 + frame.mouse_y as u32 * l.rows as u32 / GAME_H as u32;
            (col as u16, row as u16, ch)
        });
        self.cells = raster::render(&Image { w: GAME_W, h: GAME_H, px: &px }, l.mode, l.cols as usize, l.rows as usize);
        // the open inventory bar in solid quadrant blocks (David,
        // 2026-10-17: crisper icons; quadrants beat half blocks side by side)
        if let (Some((y0, y1)), Mode::Braille) = (frame.solid_band, l.mode) {
            let solid = raster::render(&Image { w: GAME_W, h: GAME_H, px: &px }, Mode::Quadrant, l.cols as usize, l.rows as usize);
            let rows = l.rows as i32;
            let r0 = (y0.max(0) * rows / GAME_H as i32).clamp(0, rows) as usize;
            let r1 = ((y1 * rows + GAME_H as i32 - 1) / GAME_H as i32).clamp(0, rows) as usize;
            let cols = l.cols as usize;
            self.cells[r0 * cols..r1 * cols].copy_from_slice(&solid[r0 * cols..r1 * cols]);
        }
        self.texts = frame
            .texts
            .iter()
            .cloned()
            .map(|mut t| {
                let i = t.color as usize * 3;
                t.color_rgb = [pal[i], pal[i + 1], pal[i + 2]];
                t
            })
            .collect();
        self.draw();

        if self.last_snapshot.elapsed() >= Duration::from_secs(1) {
            self.last_snapshot = Instant::now();
            log::debug!(
                "snapshot: mouse {},{} cursor {} texts {:?}",
                frame.mouse_x,
                frame.mouse_y,
                frame.cursor.kind,
                frame.texts.iter().map(|t| t.lines.join(" ")).collect::<Vec<_>>()
            );
        }
    }

    fn button_held(&self) -> bool {
        self.held
    }

    fn wants_audio(&self) -> bool {
        self.audio
    }

    fn menu(&mut self, title: &str, items: &[MenuItem]) -> Option<usize> {
        log::debug!("menu {title}");
        let mut sel = items.iter().position(|i| i.enabled).unwrap_or(0);
        loop {
            self.draw_menu(title, items, sel);
            let Ok(ev) = event::read() else { return None };
            match ev {
                Event::Key(k) if k.kind != KeyEventKind::Release => match k.code {
                    KeyCode::Esc | KeyCode::F(5) => return None,
                    KeyCode::Up => {
                        if let Some(p) = (0..sel).rev().find(|&i| items[i].enabled) {
                            sel = p;
                        }
                    }
                    KeyCode::Down => {
                        if let Some(n) = (sel + 1..items.len()).find(|&i| items[i].enabled) {
                            sel = n;
                        }
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        if items.get(sel).is_some_and(|i| i.enabled) {
                            log::debug!("menu {title}: chose {}", items[sel].label);
                            return Some(sel);
                        }
                    }
                    _ => {}
                },
                Event::Mouse(m) => {
                    // rows of the menu box: work out which line was hit
                    let area = self.terminal.size().unwrap_or_default();
                    let h = items.len() as u16 + 4;
                    let top = area.height.saturating_sub(h) / 2 + 2;
                    if m.row >= top && m.row < top + items.len() as u16 {
                        let i = (m.row - top) as usize;
                        if items[i].enabled {
                            sel = i;
                            if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
                                return Some(i);
                            }
                        }
                    }
                    if matches!(m.kind, MouseEventKind::Down(MouseButton::Right)) {
                        return None;
                    }
                }
                Event::Resize(..) => self.relayout(),
                _ => {}
            }
        }
    }

    fn confirm(&mut self, question: &str) -> bool {
        self.menu(question, &[MenuItem::new("Yes"), MenuItem::new("No")]) == Some(0)
    }

    fn message(&mut self, text: &str) {
        self.menu(text, &[MenuItem::new("OK")]);
    }

    fn text_input(&mut self, prompt: &str, default: &str) -> Option<String> {
        let mut s = default.to_string();
        loop {
            let shown = format!("{s}_");
            self.draw_menu(prompt, &[MenuItem::new(&format!("{shown:<30}"))], 0);
            let Ok(ev) = event::read() else { return None };
            if let Event::Key(k) = ev {
                if k.kind == KeyEventKind::Release {
                    continue;
                }
                match k.code {
                    KeyCode::Esc => return None,
                    KeyCode::Enter => return Some(s),
                    KeyCode::Backspace => {
                        s.pop();
                    }
                    KeyCode::Char(c) if s.chars().count() < 30 => s.push(c),
                    _ => {}
                }
            }
        }
    }
}
