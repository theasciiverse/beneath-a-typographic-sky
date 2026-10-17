//! `typosky walk`: plays the game headless from a plain-text script of
//! steps (`use rung`, `use rung on door`, `talk joey`, `choose shell` ...),
//! finding hotspots by the text the pointer shows over them, and prints a
//! transcript: each step, the room, and every line of text the game shows.
//!
//! Script lines (one step each; `#` starts a comment):
//!
//! - `look <thing>`: left click (look at it)
//! - `use|talk|open <thing>`: right click (use, talk); `take <thing>` and
//!   `exit|go <thing>` also check that it worked (the thing is gone, the
//!   room changed) and try again, as someone in the way can stop Foster
//! - `use <item> on <thing>`, `give <item> to <thing>`: pick the item from
//!   the inventory bar, then right click the thing with it; the thing can
//!   be another inventory item
//! - `look <item> in inventory`: left click an item in the inventory bar
//! - `choose <part of a dialogue line>`: click a line of the chooser
//! - `click <x> <y> [left|right]`: a click at screen pixels; `point <x> <y>`
//!   only moves the pointer
//! - `wait <cycles>`, `settle`, `waitfor <thing>`, `waitgone <thing>`,
//!   `waitroom <n>`, `waittext <part>`
//! - `expect room <n>`, `expect item <item>`
//! - `save <slot> <description>`: writes SKY-VM.<slot> to the save folder
//! - `frame <name>`: writes the screen as <name>.png to the frame folder
//! - `hotspots`, `inventory`, `texts`, `state [thing, ...]`: print what is
//!   there, for writing scripts; `film <cycles> <every>` prints hotspots
//!   coming and going and writes a frame every so often
//!
//! After each action the walker runs the game until the player has control
//! again and Foster has stood still for a second with nobody speaking.
//!
//! A thing is matched by its pointer text, case-insensitive: exactly
//! first, then as a part. `@name` matches the compact's own name instead
//! (`@door`), and `thing#2` takes the second match in the room.

use std::fs;
use std::path::{Path, PathBuf};

use crate::engine::defs::*;
use crate::engine::host::Headless;
use crate::engine::sky::{f, Sky};

/// Screen pixel origin in compact coordinates.
const OX: i32 = TOP_LEFT_X as i32;
const OY: i32 = TOP_LEFT_Y as i32;

/// Left (look) and right (use) as the engine numbers them.
const LEFT: u8 = 2;
const RIGHT: u8 = 1;

/// Cycles a step may take before the walker gives up.
const TIMEOUT: u32 = 3000;

#[derive(Debug, Clone)]
pub struct Hotspot {
    pub id: u16,
    pub name: String,
    pub text: String,
    /// visible rect in screen pixels, inclusive
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

pub struct Walker {
    pub sky: Sky,
    pub cycle: u64,
    /// lines shown last cycle, to log each line once when it appears
    shown: Vec<String>,
    pub saves: PathBuf,
    pub frames: Option<PathBuf>,
    pub quiet: bool,
    pub transcript: Vec<String>,
    /// stop when Foster is in the LINC
    pub until_linc: bool,
    step_no: usize,
}

/// LINC space is rooms 90 and up (the death screen is 102).
pub fn in_linc(sky: &Sky) -> bool {
    (90..100).contains(&sky.vars[SCREEN as usize])
}

impl Walker {
    pub fn new(dir: &Path) -> std::io::Result<Walker> {
        let mut sky = Sky::new(dir, Box::new(Headless::new()))?;
        sky.start(true);
        Ok(Walker {
            sky,
            cycle: 0,
            shown: Vec::new(),
            saves: PathBuf::from("walkthrough/saves"),
            frames: None,
            quiet: false,
            transcript: Vec::new(),
            until_linc: false,
            step_no: 0,
        })
    }

    fn say(&mut self, line: String) {
        if !self.quiet {
            println!("{line}");
        }
        self.transcript.push(line);
    }

    pub fn room(&self) -> u32 {
        self.sky.vars[SCREEN as usize]
    }

    // ----- one game cycle -------------------------------------------------

    pub fn tick(&mut self) {
        self.sky.tick();
        self.sky.delay(80);
        self.cycle += 1;
        let now = self.texts();
        for l in &now {
            if !self.shown.contains(l) {
                let who = if self.choosing() { "choice" } else { "text" };
                self.say(format!("    [{who}] {l}"));
            }
        }
        self.shown = now;
    }

    /// The pointer text of the hotspot under the pointer, if any.
    fn hover_text(&mut self) -> Option<String> {
        let item = self.sky.vars[SPECIAL_ITEM as usize];
        if item == 0 || item == 0xFFFF_FFFF {
            return None;
        }
        let c = self.sky.fetch(item)?;
        let t = self.sky.g(c, f::CURSOR_TEXT) as u32;
        (t != 0).then(|| String::from_utf8_lossy(&self.sky.get_text(t)).into_owned())
    }

    fn choosing(&self) -> bool {
        self.sky.sys.system_flags & SF_CHOOSING != 0
    }

    /// The texts on screen now, apart from the pointer text.
    pub fn texts(&mut self) -> Vec<String> {
        let hover = self.hover_text();
        let lines: Vec<String> = self
            .sky
            .screen
            .texts_shown
            .iter()
            .filter(|t| t.boxed.is_none())
            .map(|t| t.lines.join(" "))
            .filter(|l| Some(l) != hover.as_ref() && l != "\u{25C0}" && l != "\u{25B6}")
            .collect();
        if lines.is_empty() {
            return lines;
        }
        // the pointer text lingers a cycle after the pointer leaves: leave
        // out anything that is just the name of a hotspot here
        let names: Vec<String> = self
            .hotspots()
            .into_iter()
            .filter(|h| !(CPT_TEXT_1..=CPT_TEXT_11).contains(&(h.id as u32)))
            .map(|h| h.text)
            .collect();
        lines.into_iter().filter(|l| !names.contains(l)).collect()
    }

    // ----- hotspots -------------------------------------------------------

    /// Every hotspot the pointer can find in the current room, in the
    /// order the pointer engine tries them.
    pub fn hotspots(&mut self) -> Vec<Hotspot> {
        let room = self.room();
        let mut ids = Vec::new();
        let mut list = self.sky.vars[MOUSE_LIST_NO as usize];
        'lists: loop {
            let mut p = self.sky.cpt_start(list);
            loop {
                let id = self.sky.w(p);
                p += 1;
                if id == 0 {
                    break 'lists;
                }
                if id == 0xFFFF {
                    list = self.sky.w(p) as u32;
                    continue 'lists;
                }
                ids.push(id);
            }
        }
        let mut out = Vec::new();
        for id in ids {
            let c = self.sky.cpt_start(id as u32);
            if self.sky.g(c, f::SCREEN) as u32 != room || self.sky.g(c, f::STATUS) & ST_MOUSE as u16 == 0 {
                continue;
            }
            let left = self.sky.g(c, f::XCOOD) as i32 + self.sky.g(c, f::MOUSE_REL_X) as i16 as i32 - OX;
            let top = self.sky.g(c, f::YCOOD) as i32 + self.sky.g(c, f::MOUSE_REL_Y) as i16 as i32 - OY;
            let right = left + self.sky.g(c, f::MOUSE_SIZE_X) as i32;
            let bottom = top + self.sky.g(c, f::MOUSE_SIZE_Y) as i32;
            // a chooser line carries its text number in GET_TO_FLAG
            let text_no = if (CPT_TEXT_1..=CPT_TEXT_11).contains(&(id as u32)) {
                self.sky.g(c, f::GET_TO_FLAG) as u32
            } else {
                self.sky.g(c, f::CURSOR_TEXT) as u32
            };
            let text = if text_no == 0 { String::new() } else { String::from_utf8_lossy(&self.sky.get_text(text_no)).into_owned() };
            out.push(Hotspot {
                id,
                name: self.sky.cpt.name(id).to_string(),
                text,
                x0: left.max(0),
                y0: top.max(0),
                x1: right.min(319),
                y1: bottom.min(199),
            });
        }
        out
    }

    /// Finds a hotspot by pointer text or `@compact_name`, `#n` for the
    /// n-th match.
    fn find(&mut self, spec: &str) -> Option<Hotspot> {
        let (spec, nth) = match spec.rsplit_once('#') {
            Some((s, n)) if n.parse::<usize>().is_ok() => (s, n.parse::<usize>().unwrap().max(1) - 1),
            _ => (spec, 0),
        };
        let spec = spec.trim().to_ascii_lowercase();
        let all = self.hotspots();
        let visible = |h: &&Hotspot| h.x0 <= h.x1 && h.y0 <= h.y1;
        if let Some(name) = spec.strip_prefix('@') {
            return all.iter().filter(visible).filter(|h| h.name.to_ascii_lowercase() == name).nth(nth).cloned();
        }
        let exact: Vec<&Hotspot> = all.iter().filter(visible).filter(|h| h.text.to_ascii_lowercase() == spec).collect();
        if exact.len() > nth {
            return Some(exact[nth].clone());
        }
        all.iter()
            .filter(visible)
            .filter(|h| !h.text.is_empty() && h.text.to_ascii_lowercase().contains(&spec))
            .nth(nth)
            .cloned()
    }

    /// The hotspot the pointer engine would pick at screen pixel x, y.
    fn hit(all: &[Hotspot], x: i32, y: i32) -> Option<u16> {
        all.iter().find(|h| h.x0 <= x && x <= h.x1 && h.y0 <= y && y <= h.y1).map(|h| h.id)
    }

    /// A point inside `h` where nothing earlier in the list covers it,
    /// as near its centre as can be.
    fn aim(&mut self, h: &Hotspot) -> (u16, u16) {
        let all = self.hotspots();
        let (cx, cy) = ((h.x0 + h.x1) / 2, (h.y0 + h.y1) / 2);
        // rows 0 and 1 open the inventory bar, unless that is the target
        let ymin = if h.y1 <= 1 { 0 } else { h.y0.max(2) };
        let mut best: Option<(i32, i32, i32)> = None;
        for y in ymin..=h.y1 {
            for x in h.x0..=h.x1 {
                if Self::hit(&all, x, y) == Some(h.id) {
                    let d = (x - cx).pow(2) + (y - cy).pow(2);
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, x, y));
                    }
                }
            }
        }
        let (_, x, y) = best.unwrap_or((0, cx, cy.max(ymin)));
        (x.clamp(0, 319) as u16, y.clamp(0, 199) as u16)
    }

    /// Moves the pointer there for a cycle (so the pointer engine finds
    /// the hotspot), then clicks.
    pub fn click(&mut self, x: u16, y: u16, button: u8) {
        self.sky.mouse_moved(x, y);
        self.tick();
        self.sky.mouse_moved(x, y);
        self.sky.mouse.button_pressed(button);
        self.tick();
    }

    // ----- waiting --------------------------------------------------------

    /// The state that must stop changing before the next step.
    fn signature(&mut self) -> (u32, u16, u16, u16, u16, u32, Vec<String>) {
        let c = self.sky.cpt_start(ID_FOSTER);
        let texts = self.texts();
        let s = &self.sky;
        (
            s.vars[SCREEN as usize],
            s.g(c, f::XCOOD),
            s.g(c, f::YCOOD),
            s.g(c, f::LOGIC),
            s.g(c, f::MODE),
            s.vars[MOUSE_STATUS as usize],
            texts,
        )
    }

    fn human(&self) -> bool {
        self.sky.vars[MOUSE_STATUS as usize] & 6 == 6 && self.sky.vars[MOUSE_STOP as usize] == 0
    }

    /// Runs cycles until the player has control again and things have
    /// settled: the pointer is on, nobody is speaking and Foster has
    /// stood still for a second.
    pub fn settle(&mut self, what: &str) -> Result<(), String> {
        let mut last = self.signature();
        let mut still = 0;
        for _ in 0..TIMEOUT {
            self.tick();
            if self.sky.vars[SCREEN as usize] == 102 {
                return Err(format!("{what}: Foster died"));
            }
            let sig = self.signature();
            let speaking = !sig.6.is_empty() && !self.choosing();
            if sig == last && self.human() {
                still += 1;
                // text that stays up this long with the pointer on is a
                // window waiting for a click (the LINC's info panels)
                if (!speaking && still >= 12) || (self.choosing() && still >= 2) || still >= 150 {
                    return Ok(());
                }
            } else {
                still = 0;
            }
            last = sig;
        }
        Err(self.stuck(what))
    }

    fn stuck(&mut self, what: &str) -> String {
        let c = self.sky.cpt_start(ID_FOSTER);
        format!(
            "{what}: stuck after {TIMEOUT} cycles in room {}; texts {:?}; mouse status {} stop {}; Foster logic {} mode {} at {},{}",
            self.room(),
            self.texts(),
            self.sky.vars[MOUSE_STATUS as usize],
            self.sky.vars[MOUSE_STOP as usize],
            self.sky.g(c, f::LOGIC),
            self.sky.g(c, f::MODE),
            self.sky.g(c, f::XCOOD),
            self.sky.g(c, f::YCOOD),
        )
    }

    fn wait_until(&mut self, what: &str, done: impl FnMut(&mut Walker) -> bool) -> Result<(), String> {
        self.wait_until_for(what, TIMEOUT * 3, done)
    }

    fn wait_until_for(&mut self, what: &str, cycles: u32, mut done: impl FnMut(&mut Walker) -> bool) -> Result<(), String> {
        for _ in 0..cycles {
            if done(self) {
                return Ok(());
            }
            self.tick();
        }
        Err(self.stuck(what))
    }

    // ----- inventory ------------------------------------------------------

    /// Opens the inventory bar: the pointer to the top row.
    fn open_inventory(&mut self) {
        self.sky.mouse_moved(160, 0);
        for _ in 0..3 {
            self.tick();
        }
    }

    fn inventory_items(&mut self) -> Vec<Hotspot> {
        self.hotspots().into_iter().filter(|h| h.name.ends_with("_menu") && h.y1 < 40).collect()
    }

    /// Picks `item` out of the inventory bar, scrolling it if need be.
    fn pick(&mut self, item: &str, button: u8) -> Result<(), String> {
        self.open_inventory();
        let spec = item.to_ascii_lowercase();
        for _ in 0..12 {
            let items = self.inventory_items();
            let found = items
                .iter()
                .find(|h| h.text.to_ascii_lowercase() == spec)
                .or_else(|| items.iter().find(|h| h.text.to_ascii_lowercase().contains(&spec)))
                .cloned();
            if let Some(h) = found {
                let (x, y) = ((h.x0 + h.x1) as u16 / 2, (h.y0 + h.y1) as u16 / 2);
                if button == LEFT {
                    self.click(x, y, button);
                    return Ok(());
                }
                // the first click while the bar is still opening can be
                // lost, so click until the item is held
                for _ in 0..6 {
                    self.click(x, y, button);
                    self.tick();
                    if self.sky.vars[OBJECT_HELD as usize] == h.id as u32 {
                        return Ok(());
                    }
                }
                return Err(format!("clicking {item:?} in the inventory did not pick it up"));
            }
            // scroll right, if there is an arrow
            match self.hotspots().into_iter().find(|h| h.name == "right_arrow" && h.y1 < 40) {
                Some(a) => {
                    let (x, y) = ((a.x0 + a.x1) as u16 / 2, (a.y0 + a.y1) as u16 / 2);
                    self.click(x, y, RIGHT);
                    self.tick();
                }
                None => break,
            }
        }
        let have: Vec<String> = self.inventory_items().iter().map(|h| h.text.clone()).collect();
        Err(format!("no {item:?} in the inventory {have:?}"))
    }

    // ----- steps ----------------------------------------------------------

    fn act(&mut self, thing: &str, button: u8, what: &str) -> Result<(), String> {
        let h = self.find(thing).ok_or_else(|| self.not_found(thing))?;
        let (x, y) = self.aim(&h);
        self.say(format!("    ({} {} at {x},{y})", if button == LEFT { "look" } else { "use" }, h.name));
        // a click can go unheard (Foster still sorting himself out after
        // a lift, say): if nothing at all happens, click again
        for _ in 0..4 {
            let before = self.signature();
            self.click(x, y, button);
            for _ in 0..10 {
                let now = self.signature();
                let new_text = now.6.iter().any(|t| !before.6.contains(t) && *t != h.text);
                let moved = (now.0, now.1, now.2, now.3, now.4, now.5) != (before.0, before.1, before.2, before.3, before.4, before.5);
                if moved || new_text || self.choosing() {
                    return self.settle(what);
                }
                self.tick();
            }
            self.say("    (nothing happened, clicking again)".to_string());
        }
        self.settle(what)
    }

    fn not_found(&mut self, thing: &str) -> String {
        let here: Vec<String> = self.hotspots().iter().filter(|h| !h.text.is_empty()).map(|h| h.text.clone()).collect();
        format!("no {thing:?} in room {}; there is {here:?}", self.room())
    }

    pub fn save(&mut self, slot: u16, description: &str) -> Result<(), String> {
        fs::create_dir_all(&self.saves).map_err(|e| e.to_string())?;
        let path = self.saves.join(format!("SKY-VM.{slot:03}"));
        fs::write(&path, self.sky.prepare_save_data()).map_err(|e| e.to_string())?;
        // the slot names, as the terminal game keeps them
        let names = self.saves.join("descriptions.txt");
        let mut d: Vec<String> = fs::read_to_string(&names).unwrap_or_default().lines().map(str::to_string).collect();
        d.resize(20.max(slot as usize), String::new());
        if slot >= 1 {
            d[slot as usize - 1] = description.to_string();
        }
        fs::write(&names, d.join("\n")).map_err(|e| e.to_string())?;
        self.say(format!("    saved {} ({description})", path.display()));
        Ok(())
    }

    pub fn frame(&mut self, name: &str) {
        let Some(dir) = self.frames.clone() else { return };
        fs::create_dir_all(&dir).ok();
        let pal: Vec<u8> = self.sky.screen.sys_pal.iter().map(|&v| v >> 2).collect();
        let path = dir.join(format!("{name}.png"));
        crate::tools::write_png(&path, 320, 200, &self.sky.screen.display, &pal);
        self.say(format!("    frame {}", path.display()));
    }

    /// Runs one script line.
    pub fn step(&mut self, line: &str) -> Result<(), String> {
        let line = line.split_once('#').filter(|(_, r)| !r.starts_with(|c: char| c.is_ascii_digit())).map_or(line, |(l, _)| l).trim();
        if line.is_empty() {
            return Ok(());
        }
        self.step_no += 1;
        let room = self.room();
        self.say(format!("{:4} [room {room:3}, cycle {:6}] {line}", self.step_no, self.cycle));
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        let lower = rest.to_ascii_lowercase();
        let num = |s: &str| s.trim().parse::<u32>().map_err(|_| format!("not a number: {s:?}"));
        match verb {
            "look" if lower.ends_with(" in inventory") => {
                let item = &rest[..rest.len() - " in inventory".len()];
                self.pick(item, LEFT)?;
                self.settle(line)
            }
            "use" | "give" if lower.contains(" on ") || lower.contains(" to ") || lower.contains(" with ") => {
                let at = [" on ", " to ", " with "].iter().filter_map(|s| lower.find(s).map(|i| (i, s.len()))).min().unwrap();
                let (item, thing) = (rest[..at.0].trim(), rest[at.0 + at.1..].trim());
                self.pick(item, RIGHT)?;
                if self.sky.vars[OBJECT_HELD as usize] == 0 {
                    return Err(format!("{line}: picking {item:?} did not give a held object"));
                }
                // another inventory item: click it while the bar is open
                let target = thing.to_ascii_lowercase();
                let in_bar = self
                    .inventory_items()
                    .into_iter()
                    .find(|h| h.id as u32 != self.sky.vars[OBJECT_HELD as usize] && h.text.to_ascii_lowercase().contains(&target));
                if let Some(h) = in_bar {
                    self.say(format!("    (use on {} in the inventory)", h.name));
                    let (x, y) = ((h.x0 + h.x1) as u16 / 2, (h.y0 + h.y1) as u16 / 2);
                    self.click(x, y, RIGHT);
                    self.sky.mouse_moved(160, 100);
                    return self.settle(line);
                }
                // off the bar, so it closes, then onto the target
                self.sky.mouse_moved(160, 100);
                self.tick();
                self.tick();
                self.act(thing, RIGHT, line)
            }
            "look" => self.act(rest, LEFT, line),
            "exit" | "go" => {
                // someone in the way can stop Foster short of the exit: try
                // again until the room changes
                let room = self.room();
                for _ in 0..4 {
                    self.act(rest, RIGHT, line)?;
                    if self.room() != room || (self.until_linc && in_linc(&self.sky)) {
                        return Ok(());
                    }
                    self.say("    (still here, trying again)".to_string());
                    for _ in 0..25 {
                        self.tick();
                    }
                }
                Err(format!("{line}: still in room {room}"))
            }
            "take" | "get" => {
                // taken means gone from the room (or renamed); someone in the way can
                // stop Foster short, so try again
                for _ in 0..4 {
                    let h = self.find(rest).ok_or_else(|| self.not_found(rest))?;
                    self.act(rest, RIGHT, line)?;
                    // gone, or renamed (the wrench out of the cogs leaves "Broken cogs")
                    if !self.hotspots().iter().any(|o| o.id == h.id && o.text == h.text) {
                        return Ok(());
                    }
                    self.say("    (still there, trying again)".to_string());
                    for _ in 0..25 {
                        self.tick();
                    }
                }
                Err(format!("{line}: could not take it"))
            }
            "use" | "talk" | "open" | "press" | "push" | "pull" => self.act(rest, RIGHT, line),
            "choose" => {
                let spec = lower.clone();
                self.wait_until_for(line, 200, |w| w.choosing() && !w.choices().is_empty())?;
                let lines = self.choices();
                let h = lines
                    .iter()
                    .find(|h| h.text.to_ascii_lowercase().contains(&spec))
                    .cloned()
                    .ok_or_else(|| format!("no line {rest:?} among {:?}", lines.iter().map(|h| &h.text).collect::<Vec<_>>()))?;
                self.say(format!("    (chose {:?})", h.text));
                let (x, y) = (((h.x0 + h.x1) / 2) as u16, ((h.y0 + h.y1) / 2).max(2) as u16);
                self.click(x, y, RIGHT);
                self.settle(line)
            }
            "click" => {
                let a: Vec<&str> = rest.split_whitespace().collect();
                let (x, y) = (num(a.first().copied().unwrap_or(""))?, num(a.get(1).copied().unwrap_or(""))?);
                let b = if a.get(2) == Some(&"left") { LEFT } else { RIGHT };
                self.click(x as u16, y as u16, b);
                self.settle(line)
            }
            "point" => {
                // move the pointer only (row 0 or 1 opens the inventory bar)
                let a: Vec<&str> = rest.split_whitespace().collect();
                let (x, y) = (num(a.first().copied().unwrap_or(""))?, num(a.get(1).copied().unwrap_or(""))?);
                self.sky.mouse_moved(x as u16, y as u16);
                for _ in 0..4 {
                    self.tick();
                }
                Ok(())
            }
            "wait" => {
                for _ in 0..num(rest)? {
                    self.tick();
                }
                Ok(())
            }
            "settle" => self.settle(line),
            "state" => {
                // Foster, Joey and any hotspots named
                let mut ids = vec![ID_FOSTER, ID_JOEY];
                for spec in rest.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    let h = self.find(spec).ok_or_else(|| self.not_found(spec))?;
                    ids.push(h.id as u32);
                }
                for id in ids {
                    let c = self.sky.cpt_start(id);
                    let g = |n| self.sky.g(c, n);
                    let line = format!(
                        "    {} logic {} mode {} status {:#x} at {},{} frame {} base {:#x}:{} action {:#x}:{} request {} waiting {} screen {}",
                        self.sky.cpt.name(id as u16), g(f::LOGIC), g(f::MODE), g(f::STATUS), g(f::XCOOD), g(f::YCOOD), g(f::FRAME),
                        g(f::BASE_SUB), g(f::BASE_SUB_OFF), g(f::ACTION_SUB), g(f::ACTION_SUB_OFF), g(f::REQUEST), g(f::WAITING_FOR), g(f::SCREEN)
                    );
                    self.say(line);
                }
                let v = &self.sky.vars;
                let line = format!("    mouse status {} stop {} button {} special {}", v[MOUSE_STATUS as usize], v[MOUSE_STOP as usize], v[BUTTON as usize], v[SPECIAL_ITEM as usize]);
                self.say(line);
                Ok(())
            }
            "film" => {
                // `film <cycles> <every>`: frames and hotspot changes, for
                // finding out what happens in a room
                let a: Vec<&str> = rest.split_whitespace().collect();
                let (n, every) = (num(a.first().copied().unwrap_or(""))?, num(a.get(1).copied().unwrap_or("25"))?.max(1));
                let mut last: Vec<String> = Vec::new();
                for i in 0..n {
                    self.tick();
                    let now: Vec<String> = self.hotspots().iter().map(|h| format!("{} {:?}", h.name, h.text)).collect();
                    if now != last {
                        let gone: Vec<&String> = last.iter().filter(|h| !now.contains(h)).collect();
                        let new: Vec<&String> = now.iter().filter(|h| !last.contains(h)).collect();
                        self.say(format!("    cycle {}: room {} hotspots -{gone:?} +{new:?}", self.cycle, self.room()));
                        last = now;
                    }
                    if i % every == 0 {
                        self.frame(&format!("film-{:06}", self.cycle));
                    }
                }
                Ok(())
            }
            "waitfor" => {
                let spec = rest.to_string();
                self.wait_until(line, |w| w.find(&spec).is_some())
            }
            "waitgone" => {
                let spec = rest.to_string();
                self.wait_until(line, |w| w.find(&spec).is_none())
            }
            "waitroom" => {
                let r = num(rest)?;
                self.wait_until(line, |w| w.room() == r)?;
                self.settle(line)
            }
            "waittext" => {
                let spec = lower.clone();
                self.wait_until(line, |w| w.texts().iter().any(|t| t.to_ascii_lowercase().contains(&spec)))
            }
            "expect" => {
                let (what, arg) = rest.split_once(' ').unwrap_or((rest, ""));
                match what {
                    "room" => {
                        let r = num(arg)?;
                        if self.room() != r {
                            return Err(format!("expected room {r}, in room {}", self.room()));
                        }
                    }
                    "item" => {
                        self.open_inventory();
                        let have: Vec<String> = self.inventory_items().iter().map(|h| h.text.to_ascii_lowercase()).collect();
                        self.sky.mouse_moved(160, 100);
                        self.tick();
                        if !have.iter().any(|h| h.contains(&arg.to_ascii_lowercase())) {
                            return Err(format!("expected {arg:?} in the inventory {have:?}"));
                        }
                    }
                    _ => return Err(format!("expect what? {what:?}")),
                }
                Ok(())
            }
            "save" => {
                let (slot, desc) = rest.split_once(' ').unwrap_or((rest, ""));
                self.save(num(slot)? as u16, desc.trim())
            }
            "frame" => {
                self.frame(rest);
                Ok(())
            }
            "hotspots" => {
                for h in self.hotspots() {
                    let line = format!("    {:#06x} {:24} {:30} {},{}..{},{}", h.id, h.name, format!("{:?}", h.text), h.x0, h.y0, h.x1, h.y1);
                    self.say(line);
                }
                Ok(())
            }
            "texts" => {
                // every text sprite on screen, pointer text and labels too
                let all: Vec<String> = self
                    .sky
                    .screen
                    .texts_shown
                    .iter()
                    .map(|t| format!("    {:?} at {},{} w {} colour {}{}", t.lines.join(" / "), t.x, t.y, t.width, t.color, if t.boxed.is_some() { " (pixel button label)" } else { "" }))
                    .collect();
                for l in all {
                    self.say(l);
                }
                Ok(())
            }
            "inventory" => {
                self.open_inventory();
                let items: Vec<String> = self.inventory_items().iter().map(|h| h.text.clone()).collect();
                self.say(format!("    inventory {items:?}"));
                self.sky.mouse_moved(160, 100);
                self.tick();
                Ok(())
            }
            _ => Err(format!("unknown step {line:?}")),
        }
    }

    /// The lines of the dialogue chooser.
    fn choices(&mut self) -> Vec<Hotspot> {
        self.hotspots().into_iter().filter(|h| (CPT_TEXT_1..=CPT_TEXT_11).contains(&(h.id as u32))).collect()
    }

    /// Runs script lines in order from line `from` (1-based); stops at
    /// the first error, or in the LINC with `until_linc`. `save_at` saves
    /// into a slot once that line has run.
    pub fn run(&mut self, lines: &[String], from: usize, save_at: Option<(usize, u16)>) -> Result<(), String> {
        for (n, line) in lines.iter().enumerate().skip(from.max(1) - 1) {
            if self.until_linc && in_linc(&self.sky) && !passive(line) {
                self.say(format!("in the LINC (room {}) before line {}", self.room(), n + 1));
                return Ok(());
            }
            self.step(line).map_err(|e| format!("line {}: {e}", n + 1))?;
            if let Some((at, slot)) = save_at.filter(|&(at, _)| at == n + 1) {
                self.save(slot, &format!("walk line {at}"))?;
            }
        }
        Ok(())
    }
}

/// Steps that only look, wait or save: still run in the LINC with
/// `--until-linc`, so a script can save and take frames there.
fn passive(line: &str) -> bool {
    let verb = line.split_whitespace().next().unwrap_or("");
    line.trim_start().starts_with('#')
        || matches!(verb, "" | "save" | "frame" | "hotspots" | "inventory" | "texts" | "state" | "wait" | "settle" | "expect")
}

fn fail(msg: &str) -> ! {
    eprintln!("typosky: {msg}");
    std::process::exit(1);
}

/// `typosky walk <script> [--until-linc] [--from=<line>] [--load=<save>]
/// [--save-at=N:slot] [--frames=dir] [--saves=dir]`
pub fn walk(dir: &Path, args: &[String]) {
    let val = |name: &str| args.iter().find_map(|a| a.strip_prefix(name)?.strip_prefix('=').map(str::to_string));
    let script = args.iter().find(|a| !a.starts_with("--")).unwrap_or_else(|| fail("walk <script>"));
    let text = fs::read_to_string(script).unwrap_or_else(|e| fail(&format!("{script}: {e}")));
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut w = Walker::new(dir).unwrap_or_else(|e| fail(&e.to_string()));
    w.until_linc = args.iter().any(|a| a == "--until-linc");
    w.frames = val("--frames").map(PathBuf::from);
    if let Some(s) = val("--saves") {
        w.saves = PathBuf::from(s);
    }
    // start further on: a save to restore, and the script line to go on from
    if let Some(p) = val("--load") {
        let data = fs::read(&p).unwrap_or_else(|e| fail(&format!("{p}: {e}")));
        w.sky.restore_data(&data).unwrap_or_else(|e| fail(&format!("{p}: {e}")));
        for _ in 0..5 {
            w.tick();
        }
    }
    let from: usize = val("--from").and_then(|s| s.parse().ok()).unwrap_or(1);
    // --save-at=N:slot saves into the slot once line N has run
    let save_at: Option<(usize, u16)> = val("--save-at").and_then(|s| {
        let (n, slot) = s.split_once(':').unwrap_or((&s, "1"));
        Some((n.parse().ok()?, slot.parse().ok()?))
    });
    let result = w.run(&lines, from, save_at);
    if w.frames.is_some() {
        w.frame(&format!("end-{:06}", w.cycle));
    }
    println!("ended in room {} at cycle {}", w.room(), w.cycle);
    if let Err(e) = result {
        let here: Vec<String> = w.hotspots().iter().map(|h| format!("{} ({})", h.text, h.name)).collect();
        eprintln!("typosky walk: {e}\n  hotspots here: {here:?}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Option<PathBuf> {
        let d = std::env::var_os("TYPOSKY_DATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("reference/bass-cd-1.2"));
        d.join("sky.dsk").exists().then_some(d)
    }

    /// The walkthrough script, headless, up to the line marked
    /// `test stops here` (Anita has just given Foster the jammer).
    #[test]
    fn walkthrough_first_part() {
        let Some(dir) = data() else { return };
        let script = include_str!("../walkthrough/to-linc.txt");
        assert!(script.contains("test stops here"), "the script lost its marker");
        let lines: Vec<String> =
            script.lines().take_while(|l| !l.contains("test stops here")).map(str::to_string).collect();
        let mut w = Walker::new(&dir).unwrap();
        w.quiet = true;
        // the script's saves go somewhere harmless
        w.saves = std::env::temp_dir().join(format!("typosky-walk-test-{}", std::process::id()));
        let result = w.run(&lines, 1, None);
        std::fs::remove_dir_all(&w.saves).ok();
        if let Err(e) = result {
            panic!("{e}\n{}", w.transcript.join("\n"));
        }
        assert_eq!(w.room(), 14, "Foster should be with Anita in the reactor lobby");
    }
}
