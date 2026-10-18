//! The 320x200 frame: backgrounds, sprites, masking, palettes, fades,
//! scrolling and the swing sequences. Port of ScummVM's
//! engines/sky/screen.cpp (without the iBASS renderer).
//!
//! As in ScummVM there are two buffers: `current`, which the engine draws
//! into, and `display`, the "system" screen that dirty blocks are copied to
//! and that the host is shown.

use super::defs::*;
use super::disk::{Header, HEADER_SIZE};
use super::host::{Frame, TextOverlay};
use super::sky::{f, Sky};

pub const VGA_COLORS: usize = 256;
pub const GAME_COLORS: usize = 240;
const SCROLL_JUMP: usize = 16;
const W: usize = GAME_SCREEN_WIDTH as usize;
const H: usize = GAME_SCREEN_HEIGHT as usize;
const FULL_H: usize = FULL_SCREEN_HEIGHT as usize;
const GX: usize = GRID_X as usize;
const GY: usize = GRID_Y as usize;
const GW: usize = GRID_W as usize;
const GH: usize = GRID_H as usize;

const TOP_16_COLORS: [u8; 48] = [
    0, 0, 0, 38, 38, 38, 63, 63, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 54, 54, 54, 45, 47, 49, 32, 31, 41,
    29, 23, 37, 23, 18, 30, 49, 11, 11, 39, 5, 5, 29, 1, 1, 63, 63, 63,
];

#[derive(Default)]
pub struct SeqInfo {
    pub next_frame: u64,
    pub frames_left: u32,
    pub data: Vec<u8>,
    pub pos: usize,
    pub running: bool,
    pub running_item: bool,
}

pub struct ScreenState {
    pub current: Vec<u8>,
    scroll_screen: Option<Vec<u8>>,
    /// Screen::_palette, 0..255 per component
    pub palette: [u8; 768],
    /// the palette the host shows (the "system" palette)
    pub sys_pal: [u8; 768],
    pub display: Vec<u8>,
    pub game_grid: [u8; GX * GY],
    seq_grid: [u8; 12 * 20],
    pub seq: SeqInfo,
    spr_x: u32,
    spr_y: u32,
    spr_width: u32,
    spr_height: u32,
    mask_x1: u32,
    mask_x2: u32,
    /// text sprites drawn this cycle, for the terminal to draw as text
    pub texts_drawing: Vec<TextOverlay>,
    pub texts_shown: Vec<TextOverlay>,
    /// draw text sprites as pixels too (off when the terminal sets them as type)
    pub text_pixels: bool,
    /// pixel buttons the terminal labels in type: compact start word to label
    pub labels: std::collections::HashMap<usize, UiLabel>,
    /// the inventory bar's rows this cycle, drawn solid by the terminal
    pub band_drawing: Option<(i32, i32)>,
    pub band_shown: Option<(i32, i32)>,
}

impl Default for ScreenState {
    fn default() -> Self {
        ScreenState {
            current: vec![0; W * FULL_H],
            scroll_screen: None,
            palette: [0; 768],
            sys_pal: [0; 768],
            display: vec![0; W * FULL_H],
            game_grid: [0x80; GX * GY],
            seq_grid: [0; 12 * 20],
            seq: SeqInfo::default(),
            spr_x: 0,
            spr_y: 0,
            spr_width: 0,
            spr_height: 0,
            mask_x1: 0,
            mask_x2: 0,
            texts_drawing: Vec::new(),
            texts_shown: Vec::new(),
            text_pixels: true,
            labels: Default::default(),
            band_drawing: None,
            band_shown: None,
        }
    }
}

fn vga(v: u8) -> u8 {
    let v = v & 63;
    (v << 2) + (v >> 4)
}

impl Sky {
    pub(crate) fn screen_init(&mut self) {
        let s = &mut self.screen;
        s.game_grid = [0x80; GX * GY]; // forceRefresh
        for i in 0..VGA_COLORS - GAME_COLORS {
            for c in 0..3 {
                s.sys_pal[3 * GAME_COLORS + i * 3 + c] = vga(TOP_16_COLORS[i * 3 + c]);
            }
        }
        // our addition: each labelled button gets its own pointer text
        for &(id, label, area) in &UI_BUTTONS {
            if let Some(c) = self.fetch(id) {
                let text = match label {
                    Some(l) => l.to_string(),
                    None => {
                        let n = self.g(c, f::CURSOR_TEXT) as u32;
                        self.get_text(n).iter().map(|&b| super::text::font_char(b)).collect()
                    }
                };
                self.screen.labels.insert(c, UiLabel { text, area });
            }
        }
    }

    pub fn force_refresh(&mut self) {
        self.screen.game_grid = [0x80; GX * GY];
    }

    /// Shows the display on the host (OSystem::updateScreen).
    pub fn update_screen(&mut self) {
        let frame = Frame {
            pixels: &self.screen.display,
            palette: &self.screen.sys_pal,
            texts: &self.screen.texts_shown,
            solid_band: self.screen.band_shown,
            cursor: &self.mouse.cursor,
            mouse_x: self.mouse.x,
            mouse_y: self.mouse.y,
        };
        self.host.present(&frame);
    }

    fn copy_rect_to_display(&mut self, x: usize, y: usize, w: usize, h: usize) {
        for yy in y..(y + h).min(FULL_H) {
            let row = yy * W;
            let x1 = (x + w).min(W);
            self.screen.display[row + x..row + x1].copy_from_slice(&self.screen.current[row + x..row + x1]);
        }
    }

    fn render_final_frame(&mut self) {
        self.copy_rect_to_display(0, 0, W, H);
        self.screen.texts_shown = self.screen.texts_drawing.clone();
        self.screen.band_shown = self.screen.band_drawing;
        self.update_screen();
    }

    pub fn clear_screen(&mut self) {
        self.screen.current.iter_mut().for_each(|p| *p = 0);
        self.screen.texts_drawing.clear();
        self.render_final_frame();
    }

    /// Screen::setPalette: `pal` is 768 DOS VGA components, 0..63.
    pub fn set_palette(&mut self, pal: &[u8]) {
        for i in 0..VGA_COLORS * 3 {
            self.screen.palette[i] = vga(*pal.get(i).unwrap_or(&0));
        }
        self.screen.sys_pal[..GAME_COLORS * 3].copy_from_slice(&self.screen.palette[..GAME_COLORS * 3]);
        self.update_screen();
    }

    pub fn set_palette_file(&mut self, file: u16) {
        match self.disk.load(file) {
            Some(p) => self.set_palette(&p),
            None => log::warn!("setPalette: can't load file {file}"),
        }
    }

    pub fn halve_palette(&mut self) {
        for i in 0..GAME_COLORS * 3 {
            self.screen.sys_pal[i] = self.screen.palette[i] >> 1;
        }
    }

    /// Static images of the intro.
    pub fn show_screen_file(&mut self, file: u16, fullscreen: bool) {
        match self.disk.load(file) {
            Some(d) => {
                let s = &mut self.screen.current;
                s.iter_mut().for_each(|p| *p = 0);
                let n = d.len().min(s.len());
                s[..n].copy_from_slice(&d[..n]);
                if !fullscreen {
                    // force the last 8 lines to black
                    s[H * W..].iter_mut().for_each(|p| *p = 0);
                }
                self.render_final_frame();
            }
            None => log::warn!("showScreen: can't load file {file}"),
        }
    }

    pub fn show_current(&mut self) {
        self.render_final_frame();
    }

    /// Redraws grid blocks of layer 0 flagged for recreation.
    pub fn recreate(&mut self) {
        let layer0 = self.vars[LAYER_0_ID as usize];
        if layer0 == 0 {
            return;
        }
        let Some(data) = self.items[layer0 as usize].take() else {
            panic!("recreate: layer 0 item {layer0} not loaded");
        };
        let s = &mut self.screen;
        let mut src = 0usize;
        for by in 0..GY {
            for bx in 0..GX {
                let g = &mut s.game_grid[by * GX + bx];
                if *g & 0x80 != 0 {
                    *g &= 0x7F; // reset recreate flag
                    *g |= 1; // flag for flip
                    for y in 0..GH {
                        let at = (by * GH + y) * W + bx * GW;
                        if src + GW <= data.len() {
                            s.current[at..at + GW].copy_from_slice(&data[src..src + GW]);
                        }
                        src += GW;
                    }
                } else {
                    src += GW * GH;
                }
            }
        }
        self.items[layer0 as usize] = Some(data);
    }

    /// Copies dirty blocks to the display.
    pub fn flip(&mut self, do_update: bool) {
        for by in 0..GY {
            let mut copy_x = 0usize;
            let mut copy_w = 0usize;
            for bx in 0..GX {
                let g = &mut self.screen.game_grid[by * GX + bx];
                if *g & 1 != 0 {
                    *g &= !1;
                    if copy_w == 0 {
                        copy_x = bx * GW;
                    }
                    copy_w += GW;
                } else if copy_w != 0 {
                    self.copy_rect_to_display(copy_x, by * GH, copy_w, GH);
                    copy_w = 0;
                }
            }
            if copy_w != 0 {
                self.copy_rect_to_display(copy_x, by * GH, copy_w, GH);
            }
        }
        self.screen.texts_shown = std::mem::take(&mut self.screen.texts_drawing);
        self.screen.band_shown = self.screen.band_drawing.take();
        if do_update {
            self.update_screen();
        }
    }

    /// Screen::fnDrawScreen: set up a new room.
    pub(crate) fn screen_draw(&mut self, palette: u32, scroll: u32) {
        self.fn_fade_down(scroll);
        self.force_refresh();
        self.recreate();
        self.sprite_engine();
        self.flip(false);
        self.fn_fade_up(palette, scroll);
    }

    pub(crate) fn fn_fade_down(&mut self, scroll: u32) {
        if (scroll != 123 && scroll != 321) || self.sys.system_flags & SF_NO_SCROLL != 0 {
            let mut t = self.host.millis();
            for _ in 0..32 {
                t += 20;
                for v in self.screen.palette[..GAME_COLORS * 3].iter_mut() {
                    *v = v.saturating_sub(8);
                }
                self.screen.sys_pal[..GAME_COLORS * 3].copy_from_slice(&self.screen.palette[..GAME_COLORS * 3]);
                self.update_screen();
                self.wait_until(t);
            }
        } else {
            // scrolling is done by fnFadeUp; draw the new room aside
            let old = std::mem::replace(&mut self.screen.current, vec![0; W * FULL_H]);
            self.screen.scroll_screen = Some(old);
        }
    }

    fn wait_until(&mut self, t: u64) {
        let now = self.host.millis();
        if t > now {
            self.host.sleep((t - now) as u32);
        }
        self.pump_events();
    }

    pub fn palette_fade_up(&mut self, pal: &[u8]) {
        let mut target = [0u8; 768];
        for i in 0..768 {
            target[i] = vga(*pal.get(i).unwrap_or(&0));
        }
        let mut t = self.host.millis();
        for cnt in 1..=32u32 {
            t += 20;
            for i in 0..GAME_COLORS * 3 {
                self.screen.palette[i] = ((target[i] as u32 * cnt) >> 5) as u8;
            }
            self.screen.sys_pal[..GAME_COLORS * 3].copy_from_slice(&self.screen.palette[..GAME_COLORS * 3]);
            self.update_screen();
            self.wait_until(t);
        }
    }

    pub fn palette_fade_up_file(&mut self, file: u16) {
        match self.disk.load(file) {
            Some(p) => self.palette_fade_up(&p),
            None => log::warn!("paletteFadeUp: can't load palette {file}"),
        }
    }

    pub(crate) fn fn_fade_up(&mut self, pal_num: u32, scroll: u32) {
        let scroll = if scroll != 123 && scroll != 321 { 0 } else { scroll };
        if scroll == 0 || self.sys.system_flags & SF_NO_SCROLL != 0 {
            let pal = self.cpt_bytes(pal_num);
            self.palette_fade_up(&pal);
        } else if let Some(mut old) = self.screen.scroll_screen.take() {
            let steps = W / SCROLL_JUMP - 1;
            for cnt in 0..steps {
                for line in 0..H {
                    let row = line * W;
                    if scroll == 123 {
                        // scroll left (going right)
                        old.copy_within(row + SCROLL_JUMP..row + W, row);
                        let src = row + cnt * SCROLL_JUMP;
                        old[row + W - SCROLL_JUMP..row + W]
                            .copy_from_slice(&self.screen.current[src..src + SCROLL_JUMP]);
                    } else {
                        // scroll right (going left)
                        old.copy_within(row..row + W - SCROLL_JUMP, row + SCROLL_JUMP);
                        let src = row + W - (cnt + 1) * SCROLL_JUMP;
                        old[row..row + SCROLL_JUMP].copy_from_slice(&self.screen.current[src..src + SCROLL_JUMP]);
                    }
                }
                self.screen.display[..W * H].copy_from_slice(&old[..W * H]);
                self.update_screen();
                self.wait_for_tick();
            }
            self.render_final_frame();
        }
    }

    pub(crate) fn wait_for_tick(&mut self) {
        let start = self.host.millis();
        let end = start + 20 - (start % 20);
        loop {
            self.pump_events();
            let now = self.host.millis();
            if now >= end {
                return;
            }
            let remain = end - now;
            if remain < 10 {
                self.host.sleep(remain as u32);
                return;
            }
            self.host.sleep(10);
        }
    }

    pub(crate) fn wait_for_sequence(&mut self) {
        while self.screen.seq.running && !self.quit {
            self.process_sequence();
            self.host.sleep(20);
            self.pump_events();
        }
    }

    pub fn start_sequence(&mut self, file: u16) {
        let data = self.disk.load(file).unwrap_or_default();
        let frames = *data.first().unwrap_or(&0) as u32;
        self.screen.seq = SeqInfo {
            next_frame: self.host.millis() + 60,
            frames_left: frames,
            data,
            pos: 1,
            running: true,
            running_item: false,
        };
    }

    pub(crate) fn start_sequence_item(&mut self, item: u16) {
        let data = self.items[item as usize].clone().unwrap_or_default();
        let frames = (*data.first().unwrap_or(&1) as u32).saturating_sub(1);
        self.screen.seq = SeqInfo {
            next_frame: self.host.millis() + 60,
            frames_left: frames,
            data,
            pos: 1,
            running: true,
            running_item: true,
        };
    }

    pub fn stop_sequence(&mut self) {
        self.screen.seq.running = false;
        self.wait_for_tick();
        self.wait_for_tick();
        self.screen.seq = SeqInfo::default();
    }

    pub fn sequence_running(&self) -> bool {
        self.screen.seq.running
    }

    pub fn process_sequence(&mut self) {
        if !self.screen.seq.running {
            return;
        }
        if self.host.millis() < self.screen.seq.next_frame {
            return;
        }
        self.screen.seq.next_frame += 60;
        let s = &mut self.screen;
        s.seq_grid = [0; 12 * 20];
        let data = &s.seq.data;
        let mut p = s.seq.pos;
        let byte = |p: usize| *data.get(p).unwrap_or(&0);
        let mut pos = 0usize;
        loop {
            loop {
                let skip = byte(p);
                p += 1;
                pos += skip as usize;
                if skip != 0xFF {
                    break;
                }
            }
            loop {
                let todo = byte(p);
                p += 1;
                let sta = ((pos / (W * 16)) * 20 + ((pos % W) >> 4)).min(12 * 20 - 1);
                let end = (((pos + todo as usize) / (W * 16)) * 20 + (((pos + todo as usize) % W) >> 4))
                    .min(12 * 20 - 1);
                if end >= sta {
                    for g in sta..=end {
                        s.seq_grid[g] = 1;
                    }
                } else {
                    for g in sta..(sta / 20 + 1) * 20 {
                        s.seq_grid[g] = 1;
                    }
                    for g in (end / 20) * 20..=end {
                        s.seq_grid[g] = 1;
                    }
                }
                for _ in 0..todo {
                    if pos < s.current.len() {
                        s.current[pos] = byte(p);
                    }
                    p += 1;
                    pos += 1;
                }
                if todo != 0xFF {
                    break;
                }
            }
            if pos >= W * H {
                break;
            }
        }
        s.seq.pos = p;
        for gy in 0..12 {
            for gx in 0..20 {
                if self.screen.seq_grid[gy * 20 + gx] != 0 {
                    self.copy_rect_to_display(gx * 16, gy * 16, 16, 16);
                }
            }
        }
        self.update_screen();
        self.screen.seq.frames_left = self.screen.seq.frames_left.saturating_sub(1);
        if self.screen.seq.frames_left == 0 {
            self.screen.seq.running = false;
            self.screen.seq.data.clear();
        }
    }

    // ----- sprites.asm ---------------------------------------------------

    pub fn sprite_engine(&mut self) {
        self.screen.texts_drawing.clear();
        self.screen.band_drawing = None;
        self.do_sprites(0); // BACK
        self.sort_sprites();
        self.do_sprites(1); // FORE
    }

    fn sort_sprites(&mut self) {
        let mut draw_list_var = DRAW_LIST_NO as usize;
        while self.vars[draw_list_var] != 0 {
            let mut list_id = self.vars[draw_list_var];
            draw_list_var += 1;
            let mut sorted: Vec<(u32, usize, u32)> = Vec::with_capacity(30);
            loop {
                let mut p = self.cpt_start(list_id);
                let mut next = false;
                while !next && self.w(p) != 0 {
                    let id = self.w(p);
                    if id == 0xFFFF {
                        list_id = self.w(p + 1) as u32;
                        next = true;
                    } else {
                        let c = self.cpt_start(id as u32);
                        if self.g(c, f::STATUS) & 4 != 0 && self.g(c, f::SCREEN) as u32 == self.vars[SCREEN as usize] {
                            let item = (self.g(c, f::FRAME) >> 6) as u32;
                            match self.item(item) {
                                None => {
                                    log::debug!("missing file {item}");
                                    self.s(c, f::STATUS, 0);
                                }
                                Some(d) => {
                                    let h = Header::parse(d);
                                    let y = (self.g(c, f::YCOOD) as u32)
                                        .wrapping_add(h.offset_y as i32 as u32)
                                        .wrapping_add(h.height as u32);
                                    sorted.push((y, c, item));
                                }
                            }
                        }
                        p += 1;
                    }
                }
                if !next {
                    break;
                }
            }
            // the original bubble sort, which is stable for equal keys
            let n = sorted.len();
            if n > 1 {
                for i in 0..n - 1 {
                    for j in i + 1..n {
                        if sorted[i].0 > sorted[j].0 {
                            sorted.swap(i, j);
                        }
                    }
                }
            }
            for (_, c, item) in sorted {
                self.draw_sprite(item, c);
                let st = self.g(c, f::STATUS);
                self.vector_to_game(if st & 8 != 0 { 0x81 } else { 1 });
                if st & 0x200 == 0 {
                    self.vertical_mask();
                }
            }
        }
    }

    fn do_sprites(&mut self, layer: u8) {
        let mut draw_list_var = DRAW_LIST_NO as usize;
        while self.vars[draw_list_var] != 0 {
            let id = self.vars[draw_list_var];
            draw_list_var += 1;
            let mut p = self.cpt_start(id);
            while self.w(p) != 0 {
                while self.w(p) != 0 && self.w(p) != 0xFFFF {
                    let c = self.cpt_start(self.w(p) as u32);
                    p += 1;
                    if self.g(c, f::STATUS) & (1 << layer) != 0
                        && self.g(c, f::SCREEN) as u32 == self.vars[SCREEN as usize]
                    {
                        let item = (self.g(c, f::FRAME) >> 6) as u32;
                        if self.item(item).is_none() {
                            log::debug!("sprite data {item} not loaded");
                            self.s(c, f::STATUS, 0);
                        } else {
                            self.draw_sprite(item, c);
                            if layer == 0 {
                                self.vertical_mask();
                            }
                            let st = self.g(c, f::STATUS);
                            self.vector_to_game(if st & 8 != 0 { 0x81 } else { 1 });
                        }
                    }
                }
                while self.w(p) == 0xFFFF {
                    p = self.cpt_start(self.w(p + 1) as u32);
                }
            }
        }
    }

    fn draw_sprite(&mut self, item: u32, c: usize) {
        let Some(data) = self.items[item as usize].take() else {
            log::warn!("drawSprite: data {item} was not loaded");
            self.s(c, f::STATUS, 0);
            return;
        };
        self.draw_sprite_data(&data, c, item);
        self.items[item as usize] = Some(data);
    }

    fn draw_sprite_data(&mut self, data: &[u8], c: usize, item: u32) {
        let h = Header::parse(data);
        let s = &mut self.screen;
        s.spr_width = h.width as u32;
        s.spr_height = h.height as u32;
        s.mask_x1 = 0;
        s.mask_x2 = 0;
        let frame = self.cpt.raw[c + f::FRAME];
        let mut src = HEADER_SIZE + (frame & 0x3F) as usize * h.sp_size as usize;
        let xcood = self.cpt.raw[c + f::XCOOD] as i32;
        let ycood = self.cpt.raw[c + f::YCOOD] as i32;

        let mut spr_y = ycood + h.offset_y as i32 - TOP_LEFT_Y as i32;
        if spr_y < 0 {
            spr_y = -spr_y;
            if s.spr_height <= spr_y as u32 {
                s.spr_width = 0;
                return;
            }
            s.spr_height -= spr_y as u32;
            src += h.width as usize * spr_y as usize;
            spr_y = 0;
        } else {
            let bot_clip = H as i32 - h.height as i32 - spr_y;
            if bot_clip < 0 {
                let bot_clip = -bot_clip;
                if s.spr_height <= bot_clip as u32 {
                    s.spr_width = 0;
                    return;
                }
                s.spr_height -= bot_clip as u32;
            }
        }
        s.spr_y = spr_y as u32;
        let mut spr_x = xcood + h.offset_x as i32 - TOP_LEFT_X as i32;
        if spr_x < 0 {
            spr_x = -spr_x;
            if s.spr_width <= spr_x as u32 {
                s.spr_width = 0;
                return;
            }
            s.spr_width -= spr_x as u32;
            s.mask_x1 = spr_x as u32;
            spr_x = 0;
        } else {
            let right_clip = W as i32 - (h.width as i32 + spr_x);
            if right_clip < 0 {
                let right_clip = (-right_clip) + 1;
                if s.spr_width <= right_clip as u32 {
                    s.spr_width = 0;
                    return;
                }
                s.spr_width -= right_clip as u32;
                s.mask_x2 = right_clip as u32;
            }
        }
        s.spr_x = spr_x as u32;
        if s.spr_height > 192 || s.spr_y > 192 {
            s.spr_width = 0;
            return;
        }
        if s.spr_x + s.spr_width > 320 || s.spr_y + s.spr_height > 192 {
            log::warn!(
                "drawSprite: got x = {}, y = {}, w = {}, h = {}",
                s.spr_x,
                s.spr_y,
                s.spr_width,
                s.spr_height
            );
            s.spr_width = 0;
            return;
        }

        // A pixel button: label it for the terminal.
        if let Some(label) = s.labels.get(&c) {
            if label.text == "◀" {
                // the inventory bar is open: from the top to the arrow's foot
                let foot = ycood + h.offset_y as i32 - TOP_LEFT_Y as i32 + h.height as i32;
                s.band_drawing = Some((0, foot));
            }
            let (x, y, w, bh) = label.area.unwrap_or((
                xcood + h.offset_x as i32 - TOP_LEFT_X as i32,
                ycood + h.offset_y as i32 - TOP_LEFT_Y as i32,
                h.width,
                h.height,
            ));
            push_overlay(&mut s.texts_drawing, TextOverlay {
                x,
                y,
                width: w,
                line_height: bh,
                lines: vec![label.text.clone()],
                color: 255,
                color_rgb: [0, 0, 0],
                centered: true,
                boxed: Some(bh),
            });
        }

        // A text sprite: remember its words for the terminal.
        let is_text = (FIRST_TEXT_BUFFER..=LAST_TEXT_BUFFER).contains(&item);
        if is_text {
            if let Some(meta) = self.text.meta[(item - FIRST_TEXT_BUFFER) as usize].as_ref() {
                push_overlay(&mut s.texts_drawing, TextOverlay {
                    x: xcood + h.offset_x as i32 - TOP_LEFT_X as i32,
                    y: ycood + h.offset_y as i32 - TOP_LEFT_Y as i32,
                    width: h.width,
                    line_height: meta.line_height,
                    lines: meta.lines.clone(),
                    color: meta.color,
                    color_rgb: [0, 0, 0],
                    centered: meta.centered,
                    boxed: None,
                });
            }
        }

        let mut dst = s.spr_y as usize * W + s.spr_x as usize;
        let (mx1, mx2) = (s.mask_x1 as usize, s.mask_x2 as usize);
        let sw = s.spr_width as usize;
        let draw = s.text_pixels || !is_text;
        for _ in 0..if draw { s.spr_height } else { 0 } {
            for x in 0..sw {
                let p = *data.get(src + x + mx1).unwrap_or(&0);
                if p != 0 {
                    s.current[dst + x] = p;
                }
            }
            src += sw + mx2 + mx1;
            dst += W;
        }
        // convert to blocks for the vertical mask and vector to game
        s.spr_width += s.spr_x + GW as u32 - 1;
        s.spr_height += s.spr_y + GH as u32 - 1;
        s.spr_x >>= GRID_W_SHIFT;
        s.spr_width >>= GRID_W_SHIFT;
        s.spr_y >>= GRID_H_SHIFT;
        s.spr_height >>= GRID_H_SHIFT;
        s.spr_width -= s.spr_x;
        s.spr_height -= s.spr_y;
    }

    fn vector_to_game(&mut self, val: u8) {
        let s = &mut self.screen;
        if s.spr_width == 0 {
            return;
        }
        for y in 0..s.spr_height as usize {
            for x in 0..s.spr_width as usize {
                let i = (s.spr_y as usize + y) * GX + s.spr_x as usize + x;
                if i < s.game_grid.len() {
                    s.game_grid[i] |= val;
                }
            }
        }
    }

    fn vertical_mask(&mut self) {
        if self.screen.spr_width == 0 {
            return;
        }
        let (sx, sy, sw, sh) =
            (self.screen.spr_x as usize, self.screen.spr_y as usize, self.screen.spr_width, self.screen.spr_height);
        let start_grid = (sy + sh as usize - 1) * GX + sx;
        let start_screen = (sy + sh as usize - 1) * GH * W + sx * GW;
        for layer in LAYER_1_ID as usize..=LAYER_3_ID as usize {
            let mut grid_ofs = start_grid;
            let mut screen_pos = start_screen;
            for _ in 0..sw {
                let mut n = layer;
                while self.vars[n + 3] != 0 {
                    // note: ScummVM reads the grid of `layer`, not `n`, here
                    let grid_item = self.vars[layer + 3];
                    let hit = self
                        .item(grid_item)
                        .map(|g| le16(g, grid_ofs) != 0)
                        .unwrap_or(false);
                    if hit {
                        self.vert_mask_sub(grid_item, grid_ofs, screen_pos, layer, sh);
                        break;
                    } else {
                        n += 1;
                        if n + 3 >= self.vars.len() {
                            break;
                        }
                    }
                }
                screen_pos += GW;
                grid_ofs += 1;
            }
        }
    }

    fn vert_mask_sub(&mut self, grid_item: u32, mut grid_ofs: usize, mut screen_pos: usize, layer: usize, h: u32) {
        let layer_item = self.vars[layer];
        if grid_item == layer_item {
            return;
        }
        let Some(grid) = self.items[grid_item as usize].take() else { return };
        let Some(layer_data) = self.items[layer_item as usize].take() else {
            self.items[grid_item as usize] = Some(grid);
            return;
        };
        for _ in 0..h {
            let v = le16(&grid, grid_ofs);
            if v == 0 {
                break;
            }
            if v & 0x8000 == 0 {
                let src0 = (v as usize - 1) * GW * GH;
                for y in 0..GH {
                    for x in 0..GW {
                        let p = *layer_data.get(src0 + y * GW + x).unwrap_or(&0);
                        if p != 0 {
                            let at = screen_pos + y * W + x;
                            if at < self.screen.current.len() {
                                self.screen.current[at] = p;
                            }
                        }
                    }
                }
            }
            if screen_pos < GH * W || grid_ofs < GX {
                break;
            }
            screen_pos -= GH * W;
            grid_ofs -= GX;
        }
        self.items[grid_item as usize] = Some(grid);
        self.items[layer_item as usize] = Some(layer_data);
    }
}

/// Adds an overlay once: the game may draw the same sprite twice in a
/// cycle (the LINC panel's texts sit in two draw lists), which is
/// invisible in pixels but would double the typed text.
fn push_overlay(list: &mut Vec<TextOverlay>, o: TextOverlay) {
    if !list.contains(&o) {
        list.push(o);
    }
}

fn le16(d: &[u8], word: usize) -> u16 {
    match d.get(word * 2..word * 2 + 2) {
        Some(b) => u16::from_le_bytes([b[0], b[1]]),
        None => 0,
    }
}

/// Game objects that are buttons drawn as pixels, which the terminal
/// labels in type: the label (None takes the object's own pointer text)
/// and the screen area the whole button covers (x, y, width, height) when
/// that is more than the object's own sprite.
const UI_BUTTONS: [(u32, Option<&str>, Option<(i32, i32, u16, u16)>); 6] = [
    // the death screen's panels, measured from its background (room 102)
    (0xA2, None, Some((8, 168, 98, 24))),
    (0xA4, None, Some((107, 168, 98, 24))),
    (0xAF, None, Some((206, 168, 105, 24))),
    // the inventory bar's scroll arrows
    (0x2F, Some("◀"), None),
    (0x30, Some("▶"), None),
    // the LINC info panel's smiley button, which closes it (David,
    // 2026-10-18: "Close" reads better than a smiley at this scale)
    (0x60EA, Some("Close"), None),
];

/// A pixel button's label, and the area it covers if not its sprite's.
pub struct UiLabel {
    pub text: String,
    pub area: Option<(i32, i32, u16, u16)>,
}

pub const FIRST_TEXT_BUFFER: u32 = 274;
pub const LAST_TEXT_BUFFER: u32 = 284;
