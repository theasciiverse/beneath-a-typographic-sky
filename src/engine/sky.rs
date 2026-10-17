//! The engine state and main loop. Port of ScummVM's engines/sky/sky.cpp.
//!
//! ScummVM splits the engine into classes that hold pointers to each other
//! and share static globals (the script variables, the item list, the
//! system variables). Here all of it lives in one `Sky` struct, and each
//! of the original classes becomes an `impl Sky` block in its own file:
//! logic.rs, mcode.rs, screen.rs, mouse.rs, text.rs, grid.rs, autoroute.rs.

use std::path::Path;

use super::cpt::Cpt;
use super::defs::*;
use super::disk::Disk;
use super::host::{Action, Host, HostEvent};

pub const NUM_SKY_SCRIPTVARS: usize = 838;
pub const TOTAL_SCREENS: usize = 100;

/// Word offsets of the fields of a compact, as in ScummVM's struc.h.
pub mod f {
    pub const LOGIC: usize = 0;
    pub const STATUS: usize = 1;
    pub const SYNC: usize = 2;
    pub const SCREEN: usize = 3;
    pub const PLACE: usize = 4;
    pub const GET_TO_TABLE_ID: usize = 5;
    pub const XCOOD: usize = 6;
    pub const YCOOD: usize = 7;
    pub const FRAME: usize = 8;
    pub const CURSOR_TEXT: usize = 9;
    pub const MOUSE_ON: usize = 10;
    pub const MOUSE_OFF: usize = 11;
    pub const MOUSE_CLICK: usize = 12;
    pub const MOUSE_REL_X: usize = 13;
    pub const MOUSE_REL_Y: usize = 14;
    pub const MOUSE_SIZE_X: usize = 15;
    pub const MOUSE_SIZE_Y: usize = 16;
    pub const ACTION_SCRIPT: usize = 17;
    pub const UP_FLAG: usize = 18;
    pub const DOWN_FLAG: usize = 19;
    pub const GET_TO_FLAG: usize = 20;
    pub const FLAG: usize = 21;
    pub const MOOD: usize = 22;
    pub const GRAFIX_PROG_ID: usize = 23;
    pub const GRAFIX_PROG_POS: usize = 24;
    pub const OFFSET: usize = 25;
    pub const MODE: usize = 26;
    pub const BASE_SUB: usize = 27;
    pub const BASE_SUB_OFF: usize = 28;
    pub const ACTION_SUB: usize = 29;
    pub const ACTION_SUB_OFF: usize = 30;
    pub const GET_TO_SUB: usize = 31;
    pub const GET_TO_SUB_OFF: usize = 32;
    pub const EXTRA_SUB: usize = 33;
    pub const EXTRA_SUB_OFF: usize = 34;
    pub const DIR: usize = 35;
    pub const STOP_SCRIPT: usize = 36;
    pub const MINI_BUMP: usize = 37;
    pub const LEAVING: usize = 38;
    pub const AT_WATCH: usize = 39;
    pub const AT_WAS: usize = 40;
    pub const ALT: usize = 41;
    pub const REQUEST: usize = 42;
    pub const SP_WIDTH_XX: usize = 43;
    pub const SP_COLOR: usize = 44;
    pub const SP_TEXT_ID: usize = 45;
    pub const SP_TIME: usize = 46;
    pub const AR_ANIM_INDEX: usize = 47;
    pub const TURN_PROG_ID: usize = 48;
    pub const TURN_PROG_POS: usize = 49;
    pub const WAITING_FOR: usize = 50;
    pub const AR_TARGET_X: usize = 51;
    pub const AR_TARGET_Y: usize = 52;
    pub const ANIM_SCRATCH_ID: usize = 53;
    pub const MEGA_SET: usize = 54;
    pub const MEGA_SET_0: usize = 55;
    /// Words per mega set.
    pub const MEGA_SET_WORDS: usize = 14;

    // fields within a mega set
    pub const MS_GRID_WIDTH: usize = 0;
    pub const MS_COL_OFFSET: usize = 1;
    pub const MS_COL_WIDTH: usize = 2;
    pub const MS_LAST_CHR: usize = 3;
    pub const MS_TURN_TABLE_ID: usize = 13;
}

/// Where a script-addressable element (an original byte offset into the
/// assembly compact layout) lives.
#[derive(Clone, Copy)]
enum Elem {
    /// a word of the compact itself
    Word(usize),
    /// a word of the turn table named by mega set n's turnTableId
    Turn(usize, usize),
}

pub struct SystemVars {
    pub system_flags: u32,
    pub game_version: u32,
    pub language: u16,
    pub current_palette: u32,
    pub game_speed: u16,
    pub current_music: u16,
    pub past_intro: bool,
    pub paused: bool,
    pub seen_screen: [bool; TOTAL_SCREENS],
}

/// Thrown up the call chain when the player quits or restores, standing in
/// for ScummVM's Engine::shouldQuit polling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Quit,
}

pub struct Sky {
    pub host: Box<dyn Host>,
    pub disk: Disk,
    pub cpt: Cpt,
    pub sys: SystemVars,
    /// Logic::_scriptVariables
    pub vars: [u32; NUM_SKY_SCRIPTVARS],
    /// SkyEngine::_itemList: loaded files by number & 2047
    pub items: Vec<Option<Vec<u8>>>,

    // disk.cpp
    pub(crate) build_list: Vec<u16>,
    pub(crate) loaded_files: Vec<u32>,

    // logic.cpp
    pub(crate) modules: [Option<std::rc::Rc<Vec<u16>>>; 16],
    pub(crate) stack: Vec<u32>,
    /// start word of the compact being processed (Logic::_compact)
    pub(crate) compact: usize,
    pub(crate) object_list: [u32; 30],
    pub(crate) current_section: u32,
    pub(crate) rnd: u64,
    elem_table: Vec<Elem>,

    pub screen: super::screen::ScreenState,
    pub mouse: super::mouse::MouseState,
    pub text: super::text::TextState,
    pub grid: super::grid::Grid,
    pub route: super::autoroute::RouteState,
    pub audio: super::audio_glue::AudioGlue,

    pub(crate) fast_mode: u8,
    pub(crate) action: Action,
    pub(crate) key_pressed: bool,
    pub quit: bool,
}

impl Sky {
    pub fn new(dir: &Path, host: Box<dyn Host>) -> std::io::Result<Sky> {
        let disk = Disk::open(dir)?;
        let cpt = Cpt::open(dir, disk.game_version)?;
        let game_version = disk.game_version;
        let mut sky = Sky {
            host,
            disk,
            cpt,
            sys: SystemVars {
                system_flags: 0,
                game_version,
                language: SKY_ENGLISH as u16,
                current_palette: 4316,
                game_speed: 80,
                current_music: 0,
                past_intro: false,
                paused: false,
                seen_screen: [false; TOTAL_SCREENS],
            },
            vars: [0; NUM_SKY_SCRIPTVARS],
            items: vec![None; 300],
            build_list: vec![0; 60],
            loaded_files: vec![0; 60],
            modules: Default::default(),
            stack: Vec::with_capacity(20),
            compact: 0,
            object_list: [0; 30],
            current_section: 0xFF,
            rnd: 0x2545_F491_4F6C_DD1D,
            elem_table: build_elem_table(),
            screen: Default::default(),
            mouse: Default::default(),
            text: Default::default(),
            grid: Default::default(),
            route: Default::default(),
            audio: Default::default(),
            fast_mode: 0,
            action: Action::None,
            key_pressed: false,
            quit: false,
        };
        sky.init();
        Ok(sky)
    }

    fn init(&mut self) {
        // SkyEngine::init
        self.sys.system_flags |= SF_SBLASTER;
        if self.disk.is_cd() {
            self.sys.system_flags |= SF_ALLOW_SPEECH | SF_ALLOW_TEXT;
        } else {
            self.sys.system_flags |= SF_ALLOW_TEXT;
        }
        self.sys.system_flags |= SF_PLAY_VOCS;
        self.sys.game_speed = 80;
        self.audio_init();

        self.text_init();
        self.screen_init();
        self.mouse_init();

        // initVirgin
        self.set_palette_file(60111);
        self.show_screen_file(60110, false);

        self.load_fixed_items();
        self.logic_init();

        if !self.disk.exists(60600 + self.sys.language * 8) {
            if self.disk.exists(60600) {
                self.sys.language = SKY_ENGLISH as u16;
            } else if self.disk.exists(60600 + SKY_USA as u16 * 8) {
                self.sys.language = SKY_USA as u16;
            }
        }
    }

    fn load_fixed_items(&mut self) {
        let mut fixed = vec![49u16, 50, 73, 262];
        if !self.disk.is_demo() {
            fixed.extend_from_slice(&[36, 263, 264, 265, 266, 267, 269, 271, 272]);
        }
        for n in fixed {
            self.items[n as usize] = self.disk.load(n);
        }
    }

    pub fn item(&self, num: u32) -> Option<&Vec<u8>> {
        self.items.get(num as usize).and_then(|i| i.as_ref())
    }

    // ----- compact access -------------------------------------------------

    /// Start word of compact `id`, or None where ScummVM's fetchCpt
    /// returns NULL (id 0xFFFF, or an empty slot).
    pub fn fetch(&self, id: u32) -> Option<usize> {
        let id = id as u16;
        if id == 0xFFFF {
            return None;
        }
        self.cpt.entry(id).map(|e| e.start)
    }

    /// fetchCpt for an id that must exist; ScummVM would crash on NULL.
    pub fn cpt_start(&self, id: u32) -> usize {
        self.fetch(id).unwrap_or_else(|| panic!("compact {id:#06x} does not exist"))
    }

    #[inline]
    pub fn g(&self, c: usize, field: usize) -> u16 {
        self.cpt.raw[c + field]
    }

    #[inline]
    pub fn s(&mut self, c: usize, field: usize, v: u16) {
        self.cpt.raw[c + field] = v;
    }

    #[inline]
    pub fn w(&self, at: usize) -> u16 {
        self.cpt.raw[at]
    }

    #[inline]
    pub fn sw(&mut self, at: usize, v: u16) {
        self.cpt.raw[at] = v;
    }

    /// SkyCompact::getSub / setSub: mode is a byte offset 0..14.
    pub fn get_sub(&self, c: usize, mode: u16) -> u16 {
        assert!(mode % 2 == 0 && mode <= 14, "invalid mode {mode}");
        self.g(c, f::BASE_SUB + mode as usize / 2)
    }

    pub fn set_sub(&mut self, c: usize, mode: u16, v: u16) {
        assert!(mode % 2 == 0 && mode <= 14, "invalid mode {mode}");
        self.s(c, f::BASE_SUB + mode as usize / 2, v);
    }

    /// Start word of the compact's current mega set.
    pub fn mega_set(&self, c: usize) -> usize {
        let ms = self.g(c, f::MEGA_SET) as u32;
        let n = match ms {
            0 => 0,
            x if x == NEXT_MEGA_SET => 1,
            x if x == NEXT_MEGA_SET * 2 => 2,
            x if x == NEXT_MEGA_SET * 3 => 3,
            _ => panic!("invalid mega set {ms}"),
        };
        c + f::MEGA_SET_0 + n * f::MEGA_SET_WORDS
    }

    /// SkyCompact::getTurnTable: start word of the turn table row for `dir`.
    pub fn turn_table(&self, c: usize, dir: u16) -> usize {
        let ms = self.mega_set(c);
        let tt = self.cpt_start(self.w(ms + f::MS_TURN_TABLE_ID) as u32);
        assert!(dir <= 4, "no turn table {dir}");
        tt + dir as usize * 5
    }

    /// SkyCompact::getGrafixPtr: word index of the current graphics
    /// program position, or None.
    pub fn grafix_ptr(&self, c: usize) -> Option<usize> {
        let base = self.fetch(self.g(c, f::GRAFIX_PROG_ID) as u32)?;
        Some(base + self.g(c, f::GRAFIX_PROG_POS) as usize)
    }

    /// SkyCompact::getCompactElem: raw word index of script element `off`.
    pub fn elem(&self, c: usize, off: u16) -> usize {
        match self.elem_table.get(off as usize) {
            Some(Elem::Word(w)) => c + w,
            Some(Elem::Turn(set, w)) => {
                let tt_id = self.w(c + f::MEGA_SET_0 + set * f::MEGA_SET_WORDS + f::MS_TURN_TABLE_ID);
                self.cpt_start(tt_id as u32) + w
            }
            None => panic!("offset {off:#x} out of bounds of compact"),
        }
    }

    pub fn cpt_is_id(&self, c: usize, id: u32) -> bool {
        self.fetch(id) == Some(c)
    }

    /// Bytes of a compact (palettes are compacts).
    pub fn cpt_bytes(&self, id: u32) -> Vec<u8> {
        self.cpt.bytes(id as u16)
    }

    pub fn random(&mut self) -> u32 {
        // xorshift64*
        let mut x = self.rnd;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rnd = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }

    // ----- timing and input ----------------------------------------------

    /// SkyEngine::delay: pumps input and shows the screen while waiting.
    pub fn delay(&mut self, amount: i32) {
        let start = self.host.millis();
        self.action = Action::None;
        self.key_pressed = false;
        let amount = amount.max(0) as u64;
        loop {
            self.pump_events();
            self.update_screen();
            if amount > 0 {
                self.host.sleep(amount.min(10) as u32);
            }
            if self.host.millis() >= start + amount || self.quit {
                break;
            }
        }
    }

    pub fn pump_events(&mut self) {
        for ev in self.host.poll() {
            match ev {
                HostEvent::Action(a) => self.action = a,
                HostEvent::Key => self.key_pressed = true,
                HostEvent::MouseMove(x, y) => {
                    if self.sys.system_flags & SF_MOUSE_LOCKED == 0 {
                        self.mouse_moved(x, y);
                    }
                }
                HostEvent::Button { x, y, button } => {
                    if self.sys.system_flags & SF_MOUSE_LOCKED == 0 {
                        self.mouse_moved(x, y);
                    }
                    self.mouse.button_pressed(button);
                    self.mouse.held = true;
                }
                HostEvent::ButtonUp => self.mouse.held = false,
                HostEvent::Quit => self.quit = true,
            }
        }
    }

    fn handle_key(&mut self) {
        if (self.action != Action::None || self.key_pressed) && self.sys.paused {
            self.audio_unpause_fx();
            self.sys.paused = false;
            let pal = self.cpt_bytes(self.sys.current_palette);
            self.set_palette(&pal);
        } else {
            match self.action {
                Action::ToggleFastMode => self.fast_mode ^= 1,
                Action::ToggleReallyFastMode => self.fast_mode ^= 2,
                Action::OpenControlPanel => self.do_control_panel(),
                Action::Skip => {
                    if !self.sys.past_intro {
                        self.restart_game();
                    }
                }
                Action::SkipLine => self.mouse.logic_click = true,
                Action::Pause => {
                    self.halve_palette();
                    self.audio_pause_fx();
                    self.sys.paused = true;
                }
                Action::None => {}
            }
        }
        self.action = Action::None;
        self.key_pressed = false;
    }

    /// SkyEngine::go: the intro, then the game loop until quit.
    pub fn run(&mut self, skip_intro: bool) {
        self.start(skip_intro);
        let mut delay_count = self.host.millis() as i64;
        while !self.quit {
            self.tick();
            if self.fast_mode & 2 != 0 {
                self.delay(0);
            } else if self.fast_mode & 1 != 0 {
                self.delay(10);
            } else {
                let speed = self.sys.game_speed as i64;
                delay_count += speed;
                let mut need = delay_count - self.host.millis() as i64;
                if need < 0 || need > speed {
                    need = 0;
                    delay_count = self.host.millis() as i64;
                }
                self.delay(need as i32);
            }
        }
        self.audio_stop_music();
    }

    /// Everything in SkyEngine::go before the game loop.
    pub fn start(&mut self, skip_intro: bool) {
        self.sys.past_intro = false;
        let intro_skipped = if skip_intro || self.sys.game_version <= 272 {
            true
        } else {
            !self.do_intro()
        };
        if self.quit {
            return;
        }
        self.clear_screen();
        if intro_skipped {
            self.restart_game();
        } else {
            self.init_screen0();
        }
    }

    /// One game cycle (80 ms at normal speed), without the wait.
    pub fn tick(&mut self) {
        self.audio_check_fx_queue();
        self.mouse_engine();
        self.handle_key();
        if self.sys.paused {
            loop {
                self.update_screen();
                self.delay(50);
                self.handle_key();
                if !self.sys.paused || self.quit {
                    break;
                }
            }
        }
        self.logic_engine();
        self.process_sequence();
        self.recreate();
        self.sprite_engine();
        self.flip(true);
    }
}

/// Builds the table behind getCompactElem from the original layout: each
/// 16-bit field takes two byte offsets, each 32-bit field (a pointer in the
/// assembly original) four; the extra offsets point at field 0, as the
/// zero entries in ScummVM's offset tables do.
fn build_elem_table() -> Vec<Elem> {
    let mut t = Vec::new();
    let push16 = |t: &mut Vec<Elem>, e: Elem| {
        t.push(e);
        t.push(Elem::Word(0));
    };
    let push32 = |t: &mut Vec<Elem>, e: Elem| {
        t.push(e);
        t.push(Elem::Word(0));
        t.push(Elem::Word(0));
        t.push(Elem::Word(0));
    };
    // Compact: (word index, is 32-bit in the original)
    let compact: &[(usize, bool)] = &[
        (f::LOGIC, false),
        (f::STATUS, false),
        (f::SYNC, false),
        (f::SCREEN, false),
        (f::PLACE, false),
        (f::GET_TO_TABLE_ID, true),
        (f::XCOOD, false),
        (f::YCOOD, false),
        (f::FRAME, false),
        (f::CURSOR_TEXT, false),
        (f::MOUSE_ON, false),
        (f::MOUSE_OFF, false),
        (f::MOUSE_CLICK, false),
        (f::MOUSE_REL_X, false),
        (f::MOUSE_REL_Y, false),
        (f::MOUSE_SIZE_X, false),
        (f::MOUSE_SIZE_Y, false),
        (f::ACTION_SCRIPT, false),
        (f::UP_FLAG, false),
        (f::DOWN_FLAG, false),
        (f::GET_TO_FLAG, false),
        (f::FLAG, false),
        (f::MOOD, false),
        (f::GRAFIX_PROG_ID, true),
        (f::OFFSET, false),
        (f::MODE, false),
        (f::BASE_SUB, false),
        (f::BASE_SUB_OFF, false),
        (f::ACTION_SUB, false),
        (f::ACTION_SUB_OFF, false),
        (f::GET_TO_SUB, false),
        (f::GET_TO_SUB_OFF, false),
        (f::EXTRA_SUB, false),
        (f::EXTRA_SUB_OFF, false),
        (f::DIR, false),
        (f::STOP_SCRIPT, false),
        (f::MINI_BUMP, false),
        (f::LEAVING, false),
        (f::AT_WATCH, false),
        (f::AT_WAS, false),
        (f::ALT, false),
        (f::REQUEST, false),
        (f::SP_WIDTH_XX, false),
        (f::SP_COLOR, false),
        (f::SP_TEXT_ID, false),
        (f::SP_TIME, false),
        (f::AR_ANIM_INDEX, false),
        (f::TURN_PROG_ID, true),
        (f::WAITING_FOR, false),
        (f::AR_TARGET_X, false),
        (f::AR_TARGET_Y, false),
        (f::ANIM_SCRATCH_ID, true),
        (f::MEGA_SET, false),
    ];
    for &(w, is32) in compact {
        if is32 {
            push32(&mut t, Elem::Word(w));
        } else {
            push16(&mut t, Elem::Word(w));
        }
    }
    debug_assert_eq!(t.len() as u32, C_GRID_WIDTH);
    for set in 0..4 {
        let base = f::MEGA_SET_0 + set * f::MEGA_SET_WORDS;
        for w in 0..4 {
            push16(&mut t, Elem::Word(base + w));
        }
        for w in 4..13 {
            push32(&mut t, Elem::Word(base + w));
        }
        for w in 0..25 {
            push32(&mut t, Elem::Turn(set, w));
        }
    }
    t
}
