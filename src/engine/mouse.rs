//! The mouse: hotspots under the pointer, clicks, pointer shapes. Port of
//! the classic (non-iBASS) parts of ScummVM's engines/sky/mouse.cpp.

use super::defs::*;
use super::disk::{Header, HEADER_SIZE};
use super::host::Cursor;
use super::sky::{f, Sky};

const MICE_FILE: u16 = 60300;

const MOUSE_MAIN_OBJECTS: [u32; 24] =
    [65, 9, 66, 64, 8, 63, 10, 11, 71, 76, 37, 36, 42, 75, 79, 6, 74, 39, 49, 43, 34, 35, 77, 38];

const MOUSE_LINC_OBJECTS: [u32; 21] = [
    24625, 24649, 24827, 24651, 24583, 24581, 24582, 24628, 24650, 24629, 24732, 24631, 24584, 24630, 24626,
    24627, 24632, 24643, 24828, 24830, 24829,
];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum MouseMode {
    #[default]
    Gameplay,
    TextChooser,
    MustRelease,
}

#[derive(Default)]
pub struct MouseState {
    pub x: u16,
    pub y: u16,
    pub b: u16,
    pub logic_click: bool,
    pub held: bool,
    pub current_cursor: u16,
    mice_data: Vec<u8>,
    object_mouse_data: Vec<u8>,
    pub cursor: Cursor,
    mode: MouseMode,
}

impl MouseState {
    pub fn button_pressed(&mut self, button: u8) {
        self.b = button as u16;
    }

    pub fn set_text_chooser_mode(&mut self) {
        self.mode = MouseMode::TextChooser;
    }
}

impl Sky {
    pub(crate) fn mouse_init(&mut self) {
        let m = &mut self.mouse;
        m.b = 0;
        m.current_cursor = 6;
        m.x = GAME_SCREEN_WIDTH as u16 / 2;
        m.y = GAME_SCREEN_HEIGHT as u16 / 2;
        m.mice_data = self.disk.load(MICE_FILE).unwrap_or_default();
        m.object_mouse_data = self.disk.load(MICE_FILE + 1).unwrap_or_default();
    }

    pub(crate) fn replace_mouse_cursors(&mut self, file: u16) {
        self.mouse.object_mouse_data = self.disk.load(file).unwrap_or_default();
    }

    pub fn mouse_moved(&mut self, x: u16, y: u16) {
        self.mouse.x = x;
        self.mouse.y = y;
    }

    pub(crate) fn fn_add_human(&mut self) -> bool {
        // give the human control of the player again
        if self.mouse.mode == MouseMode::Gameplay {
            self.mouse.mode = MouseMode::MustRelease;
        }
        if self.vars[MOUSE_STOP as usize] == 0 {
            self.vars[MOUSE_STATUS as usize] |= 6; // cursor & mouse
            if self.mouse.y < 2 {
                // stop the mouse activating the top line
                self.mouse.y = 2;
            }
            // force the pointer engine to run a get-off even over nothing;
            // the get-off may remove pointer text
            let get_off = self.vars[GET_OFF as usize];
            if get_off != 0 {
                self.script((get_off & 0xFFFF) as u16, (get_off >> 16) as u16);
            }
            self.vars[SPECIAL_ITEM as usize] = 0xFFFF_FFFF;
            self.vars[GET_OFF as usize] = RESET_MOUSE;
        }
        true
    }

    pub(crate) fn sprite_mouse(&mut self, frame: u16, hot_x: u8, hot_y: u8) -> bool {
        self.mouse.current_cursor = frame;
        let data = &self.mouse.mice_data;
        if data.len() < HEADER_SIZE {
            return true;
        }
        let h = Header::parse(data);
        let start = HEADER_SIZE + h.sp_size as usize * frame as usize;
        let len = h.width as usize * h.height as usize;
        let pixels = data.get(start..start + len).map(|p| p.to_vec()).unwrap_or_default();
        self.mouse.cursor = Cursor {
            visible: frame as u32 != MOUSE_BLANK,
            width: h.width,
            height: h.height,
            hot_x: hot_x as u16,
            hot_y: hot_y as u16,
            pixels,
            kind: frame,
        };
        true
    }

    /// One game cycle of the mouse (Mouse::mouseEngine).
    pub(crate) fn mouse_engine(&mut self) {
        // the click signal is available to Logic for one game cycle
        self.mouse.logic_click = self.mouse.b > 0;
        if self.vars[MOUSE_STOP as usize] == 0 && self.vars[MOUSE_STATUS as usize] & (1 << 1) != 0 {
            let (x, y) = (self.mouse.x + TOP_LEFT_X as u16, self.mouse.y + TOP_LEFT_Y as u16);
            self.pointer_engine(x, y);
            if self.vars[MOUSE_STATUS as usize] & (1 << 2) != 0 {
                self.button_engine1();
            }
        }
        self.mouse.b = 0; // don't save up buttons
    }

    fn pointer_engine(&mut self, x: u16, y: u16) {
        let mut list_num = self.vars[MOUSE_LIST_NO as usize];
        loop {
            let mut p = self.cpt_start(list_num);
            while self.w(p) != 0 && self.w(p) != 0xFFFF {
                let item_num = self.w(p);
                let c = self.cpt_start(item_num as u32);
                p += 1;
                if self.g(c, f::SCREEN) as u32 == self.vars[SCREEN as usize] && self.g(c, f::STATUS) & 16 != 0 {
                    let left = self.g(c, f::XCOOD) as i32 + self.g(c, f::MOUSE_REL_X) as i16 as i32;
                    let top = self.g(c, f::YCOOD) as i32 + self.g(c, f::MOUSE_REL_Y) as i16 as i32;
                    let (x, y) = (x as i32, y as i32);
                    if left > x || left + (self.g(c, f::MOUSE_SIZE_X) as i32) < x {
                        continue;
                    }
                    if top > y || top + (self.g(c, f::MOUSE_SIZE_Y) as i32) < y {
                        continue;
                    }
                    // we've hit the item
                    if self.vars[SPECIAL_ITEM as usize] == item_num as u32 {
                        return;
                    }
                    self.vars[SPECIAL_ITEM as usize] = item_num as u32;
                    let get_off = self.vars[GET_OFF as usize];
                    if get_off != 0 {
                        self.mouse_script(get_off, c);
                    }
                    self.vars[GET_OFF as usize] = self.g(c, f::MOUSE_OFF) as u32;
                    let on = self.g(c, f::MOUSE_ON);
                    if on != 0 {
                        self.mouse_script(on as u32, c);
                    }
                    return;
                }
            }
            if self.w(p) == 0xFFFF {
                list_num = self.w(p + 1) as u32;
                continue;
            }
            break;
        }
        // over nothing
        if self.vars[SPECIAL_ITEM as usize] != 0 {
            self.vars[SPECIAL_ITEM as usize] = 0;
            let get_off = self.vars[GET_OFF as usize];
            if get_off != 0 {
                self.script((get_off & 0xFFFF) as u16, (get_off >> 16) as u16);
            }
            self.vars[GET_OFF as usize] = 0;
        }
    }

    fn button_engine1(&mut self) {
        // checks for clicking on a special item
        if self.mouse.b != 0 {
            self.vars[BUTTON as usize] = self.mouse.b as u32;
            let special = self.vars[SPECIAL_ITEM as usize];
            if special != 0 && special != 0xFFFF_FFFF {
                let item = self.cpt_start(special);
                let click = self.g(item, f::MOUSE_CLICK);
                if click != 0 {
                    self.mouse_script(click as u32, item);
                }
            }
        }
    }

    fn find_mouse_cursor(item: u32) -> u16 {
        if let Some(i) = MOUSE_MAIN_OBJECTS.iter().position(|&o| o == item) {
            return i as u16;
        }
        if let Some(i) = MOUSE_LINC_OBJECTS.iter().position(|&o| o == item) {
            return i as u16;
        }
        0
    }

    pub(crate) fn fn_open_close_hand(&mut self, open: bool) -> bool {
        let held = self.vars[OBJECT_HELD as usize];
        if !open && held == 0 {
            self.sprite_mouse(1, 0, 0);
            return true;
        }
        let mut cursor = Self::find_mouse_cursor(held) << 1;
        if open {
            cursor += 1;
        }
        // copy the object's pointer into mouse frame 0
        let od = &self.mouse.object_mouse_data;
        if od.len() >= HEADER_SIZE && self.mouse.mice_data.len() >= HEADER_SIZE {
            let size = Header::parse(od).sp_size as usize;
            let src = HEADER_SIZE + size * cursor as usize;
            if let Some(s) = od.get(src..src + size) {
                let s = s.to_vec();
                let end = (HEADER_SIZE + size).min(self.mouse.mice_data.len());
                self.mouse.mice_data[HEADER_SIZE..end].copy_from_slice(&s[..end - HEADER_SIZE]);
            }
        }
        self.sprite_mouse(0, 5, 5);
        true
    }

    pub(crate) fn mouse_was_clicked(&mut self) -> bool {
        std::mem::take(&mut self.mouse.logic_click)
    }

    pub(crate) fn wait_mouse_not_pressed(&mut self, min_delay: u32) {
        let start = self.host.millis();
        let mut min_delay = min_delay as u64;
        let mut pressed = true;
        while (pressed || self.host.millis() < start + min_delay) && !self.quit {
            if !self.mouse.held && !self.host.button_held() {
                pressed = false;
            }
            self.pump_events();
            if self.action == super::host::Action::Skip {
                min_delay = 0;
                pressed = false;
            }
            self.update_screen();
            self.host.sleep(20);
        }
    }
}
