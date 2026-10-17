//! The mcode functions, called from scripts. Port of the fn* functions of
//! ScummVM's engines/sky/logic.cpp. Each returns false to make the calling
//! script drop out (to be resumed next cycle), true to keep going.

use super::defs::*;
use super::disk::Header;
use super::sky::{f, Sky};
use super::text::Align;

impl Sky {
    pub(crate) fn mcode(&mut self, n: u16, a: u32, b: u32, c: u32) -> bool {
        match n {
            0 => self.fn_cache_chip(a),
            1 => self.fn_cache_fast(a),
            2 => self.fn_draw_screen(a, b),
            3 => self.fn_ar(a, b),
            4 => self.fn_ar_animate(),
            5 => self.fn_idle(),
            6 => self.fn_interact(a),
            7 => self.fn_start_sub(a),
            8 => self.fn_they_start_sub(a, b),
            9 => self.fn_assign_base(a, b),
            10 => self.sprite_mouse(MOUSE_DISK as u16, 11, 11),
            11 => self.sprite_mouse(MOUSE_NORMAL as u16, 0, 0),
            12 => self.fn_blank_mouse(),
            13 => self.fn_cross_mouse(a, b, c),
            14 => self.sprite_mouse(MOUSE_RIGHT as u16, 9, 4),
            15 => self.sprite_mouse(MOUSE_LEFT as u16, 0, 5),
            16 => self.sprite_mouse(MOUSE_DOWN as u16, 9, 4),
            17 => self.fn_open_close_hand(true),
            18 => self.fn_open_close_hand(false),
            19 => self.fn_get_to(a, b),
            20 => self.fn_set_to_stand(),
            21 => self.fn_turn_to(a),
            22 => self.fn_arrived(a),
            23 => self.fn_leaving(),
            24 => self.fn_set_alternate(a),
            25 => self.fn_alt_set_alternate(a, b),
            26 => self.fn_kill_id(a),
            27 => self.fn_no_human(a, b, c),
            28 => self.fn_add_human(),
            29 => {
                self.vars[MOUSE_STATUS as usize] |= 4;
                true
            }
            30 => {
                self.vars[MOUSE_STATUS as usize] &= 0xFFFF_FFFB;
                true
            }
            31 => {
                self.vars[MOUSE_STOP as usize] |= 1;
                true
            }
            32 => {
                self.vars[MOUSE_STOP as usize] = 0;
                true
            }
            33 => {
                let (x, y) = (self.mouse.x, self.mouse.y);
                self.fn_pointer_text(a, x, y);
                true
            }
            34 => false, // fnQuit
            35 => self.fn_speak_me(a, b, c),
            36 => self.fn_speak_me_dir(a, b, c),
            37 => self.fn_speak_wait(a, b, c),
            38 => self.fn_speak_wait_dir(a, b, c),
            39 => self.fn_chooser(),
            40 => self.fn_highlight(a, b),
            41 => self.fn_text_kill(),
            42 => {
                let cp = self.compact;
                self.s(cp, f::LOGIC, L_STOPPED as u16);
                false
            }
            43 => self.fn_we_wait(a),
            44 => {
                let t = self.cpt_start(a);
                self.s(t, f::SYNC, b as u16);
                false
            }
            45 => {
                let t = self.cpt_start(a);
                self.s(t, f::SYNC, b as u16);
                true
            }
            46 => {
                let t = self.cpt_start(a);
                self.s(t, f::REQUEST, b as u16);
                false
            }
            47 => {
                let t = self.cpt_start(a);
                self.s(t, f::REQUEST, 0);
                true
            }
            48 => self.fn_check_request(),
            49 => self.fn_start_menu(a),
            50 => {
                let t = self.cpt_start(a);
                let fr = self.g(t, f::FRAME).wrapping_sub(1);
                self.s(t, f::FRAME, fr);
                self.s(t, f::GET_TO_FLAG, 0);
                true
            }
            51 => self.fn_face_id(a),
            52 => self.set_sprite_kind(Some(a), ST_FOREGROUND),
            53 => self.set_sprite_kind(None, ST_BACKGROUND),
            54 => self.set_sprite_kind(Some(a), ST_BACKGROUND),
            55 => self.set_sprite_kind(Some(a), ST_SORT),
            56 => self.set_sprite_kind(None, 0),
            57 => self.set_sprite_kind(Some(a), 0),
            58 => self.fn_reset_id(a, b),
            59 => {
                let cp = self.compact;
                let st = self.g(cp, f::STATUS) ^ ST_GRID_PLOT as u16;
                self.s(cp, f::STATUS, st);
                true
            }
            60 => {
                let cp = self.compact;
                self.s(cp, f::FLAG, a as u16);
                self.s(cp, f::LOGIC, L_PAUSE as u16);
                false
            }
            61 => self.fn_run_anim_mod(a),
            62 => self.fn_simple_mod(a, L_SIMPLE_MOD),
            63 => self.fn_simple_mod(a, L_FRAMES),
            64 => {
                let cp = self.compact;
                if self.g(cp, f::SYNC) != 0 {
                    return true;
                }
                self.s(cp, f::LOGIC, L_WAIT_SYNC as u16);
                false
            }
            65 => {
                let cp = self.compact;
                let m = self.g(cp, f::MEGA_SET).wrapping_add(NEXT_MEGA_SET as u16);
                self.s(cp, f::MEGA_SET, m);
                true
            }
            66 => {
                let cp = self.compact;
                let m = self.g(cp, f::MEGA_SET).wrapping_sub(NEXT_MEGA_SET as u16);
                self.s(cp, f::MEGA_SET, m);
                true
            }
            67 => {
                let t = self.cpt_start(a);
                self.s(t, f::MEGA_SET, (b * NEXT_MEGA_SET) as u16);
                true
            }
            68 => self.fn_move_items(a, b),
            69 => {
                for i in 0..16 {
                    self.vars[TEXT1 as usize + i] = 0;
                }
                true
            }
            70 => {
                // fnAskThis: find the first free position
                let mut p = TEXT1 as usize;
                while self.vars[p] != 0 {
                    p += 2;
                }
                self.vars[p] = a;
                self.vars[p + 1] = b;
                true
            }
            71 => {
                let r = self.random() % 65536;
                self.vars[RND as usize] = r & a;
                true
            }
            72 => {
                let t = self.cpt_start(a);
                self.vars[RESULT as usize] = (self.g(t, f::SCREEN) as u32 == b) as u32;
                true
            }
            73 => {
                let t = self.cpt_start(a);
                let st = self.g(t, f::STATUS) ^ ST_MOUSE as u16;
                self.s(t, f::STATUS, st);
                true
            }
            74 => {
                let t = self.cpt_start(a);
                let st = self.g(t, f::STATUS) | ST_MOUSE as u16;
                self.s(t, f::STATUS, st);
                true
            }
            75 => {
                let t = self.cpt_start(a);
                let st = self.g(t, f::STATUS) & !(ST_MOUSE as u16);
                self.s(t, f::STATUS, st);
                true
            }
            76 => {
                let t = self.cpt_start(a);
                self.vars[RESULT as usize] = self.g(t, f::XCOOD) as u32;
                true
            }
            77 => {
                let t = self.cpt_start(a);
                self.vars[RESULT as usize] = self.g(t, f::YCOOD) as u32;
                true
            }
            78 => self.fn_test_list(a, b, c),
            79 => {
                let t = self.cpt_start(a);
                self.vars[RESULT as usize] = self.g(t, f::PLACE) as u32;
                true
            }
            80 => {
                // fnCustomJoey
                let t = self.cpt_start(a);
                self.vars[PLAYER_X as usize] = self.g(t, f::XCOOD) as u32;
                self.vars[PLAYER_Y as usize] = self.g(t, f::YCOOD) as u32;
                self.vars[PLAYER_MOOD as usize] = self.g(t, f::MOOD) as u32;
                self.vars[PLAYER_SCREEN as usize] = self.g(t, f::SCREEN) as u32;
                true
            }
            81 => {
                let pal = self.cpt_bytes(a);
                self.set_palette(&pal);
                self.sys.current_palette = a;
                true
            }
            82 => {
                self.fn_text_module(a, b);
                true
            }
            83 => {
                let t = self.cpt_start(a);
                self.s(t, f::CURSOR_TEXT, b as u16);
                true
            }
            84 => {
                self.fn_mini_load(a as u16);
                true
            }
            85 => {
                self.fn_flush_buffers();
                true
            }
            86 => true, // fnFlushChip: done automatically
            87 => {
                self.vars[SAFEX as usize] = self.mouse.x as u32 + TOP_LEFT_X;
                self.vars[SAFEY as usize] = self.mouse.y as u32 + TOP_LEFT_Y;
                true
            }
            88 => {
                let cp = self.compact;
                self.plot_grid(a, b, c, cp);
                true
            }
            89 => {
                let cp = self.compact;
                self.remove_grid(a, b, c, cp);
                true
            }
            90 => self.fn_eyeball(a),
            91 => self.sprite_mouse(MOUSE_UP as u16, 9, 4),
            92 => self.fn_leave_section(a, b, c),
            93 => self.fn_enter_section(a, b, c),
            94 => {
                self.do_load_save_panel();
                false
            }
            95 => {
                self.restart_game();
                false
            }
            96 => {
                // fnNewSwingSeq: only certain files work on the pc
                if a == 85 || a == 106 || a == 75 || a == 15 {
                    self.start_sequence_item(a as u16);
                } else {
                    log::debug!("fnNewSwingSeq: ignored seq {a}");
                }
                true
            }
            97 => {
                self.wait_for_sequence();
                true
            }
            98 => {
                self.sys.past_intro = true;
                true
            }
            99 => {
                self.clear_screen();
                true
            }
            100 => self.fn_print_credit(a, c),
            101 => self.fn_look_at(a, c),
            102 => self.fn_linc_text_module(a, b, c),
            103 => {
                // fnTextKill2: kill all text items
                for id in FIRST_TEXT_COMPACT..FIRST_TEXT_COMPACT + 10 {
                    let t = self.cpt_start(id);
                    self.s(t, f::STATUS, 0);
                }
                true
            }
            104 => {
                self.fn_set_font(a);
                true
            }
            105 => self.fn_start_fx(a, b, c),
            106 => {
                self.audio_stop_fx();
                true
            }
            107 => {
                if self.sys.system_flags & SF_MUS_OFF == 0 {
                    self.audio_start_music(a as u16);
                }
                self.sys.current_music = a as u16;
                true
            }
            108 => {
                self.audio_start_music(0);
                self.sys.current_music = 0;
                true
            }
            109 => {
                self.fn_fade_down(a);
                true
            }
            110 => {
                self.sys.current_palette = a;
                self.fn_fade_up(a, b);
                true
            }
            111 => {
                self.quit = true;
                false
            }
            112 => {
                self.audio_pause_fx();
                true
            }
            113 => {
                self.audio_unpause_fx();
                true
            }
            114 => {
                log::debug!("fnPrintf({a}, {b}, {c})");
                true
            }
            n => panic!("unknown mcode {n}"),
        }
    }

    fn fn_cache_chip(&mut self, a: u32) -> bool {
        self.audio_stop_fx();
        let list = self.cpt_start(a);
        self.disk_cache_chip(list);
        true
    }

    fn fn_cache_fast(&mut self, a: u32) -> bool {
        let list = self.fetch(a);
        self.disk_cache_fast(list);
        true
    }

    fn fn_draw_screen(&mut self, a: u32, b: u32) -> bool {
        self.sys.current_palette = a;
        self.screen_draw(a, b);
        if self.vars[SCREEN as usize] == 32 {
            // workaround for script bug #1140: the gardener can get stuck
            // waiting for a sync from Foster; send one on every entry
            let g = self.cpt_start(ID_SC32_GARDENER);
            self.s(g, f::SYNC, 1);
        }
        true
    }

    fn fn_ar(&mut self, x: u32, y: u32) -> bool {
        let c = self.compact;
        self.s(c, f::DOWN_FLAG, 1); // assume failure in case speech interrupts
        self.s(c, f::AR_TARGET_X, x as u16);
        self.s(c, f::AR_TARGET_Y, y as u16);
        self.s(c, f::LOGIC, L_AR as u16);
        let xc = self.g(c, f::XCOOD) & 0xFFF8;
        let yc = self.g(c, f::YCOOD) & 0xFFF8;
        self.s(c, f::XCOOD, xc);
        self.s(c, f::YCOOD, yc);
        false
    }

    fn fn_ar_animate(&mut self) -> bool {
        let c = self.compact;
        self.s(c, f::MOOD, 0); // not stood still
        self.s(c, f::LOGIC, L_AR_ANIM as u16);
        false
    }

    fn fn_idle(&mut self) -> bool {
        let c = self.compact;
        self.s(c, f::LOGIC, 0);
        true
    }

    fn fn_interact(&mut self, target: u32) -> bool {
        let c = self.compact;
        let m = self.g(c, f::MODE) + 4;
        self.s(c, f::MODE, m);
        self.s(c, f::LOGIC, L_SCRIPT as u16);
        let t = self.cpt_start(target);
        let action = self.g(t, f::ACTION_SCRIPT);
        self.set_sub(c, m, action);
        self.set_sub(c, m + 2, 0);
        false
    }

    fn fn_start_sub(&mut self, scr: u32) -> bool {
        let c = self.compact;
        let m = self.g(c, f::MODE) + 4;
        self.s(c, f::MODE, m);
        self.set_sub(c, m, (scr & 0xFFFF) as u16);
        self.set_sub(c, m + 2, (scr >> 16) as u16);
        false
    }

    fn fn_they_start_sub(&mut self, mega: u32, scr: u32) -> bool {
        let t = self.cpt_start(mega);
        let m = self.g(t, f::MODE) + 4;
        self.s(t, f::MODE, m);
        self.set_sub(t, m, (scr & 0xFFFF) as u16);
        self.set_sub(t, m + 2, (scr >> 16) as u16);
        true
    }

    fn fn_assign_base(&mut self, id: u32, scr: u32) -> bool {
        let t = self.cpt_start(id);
        self.s(t, f::MODE, C_BASE_MODE as u16);
        self.s(t, f::LOGIC, L_SCRIPT as u16);
        self.s(t, f::BASE_SUB, (scr & 0xFFFF) as u16);
        self.s(t, f::BASE_SUB_OFF, (scr >> 16) as u16);
        true
    }

    fn fn_blank_mouse(&mut self) -> bool {
        self.sprite_mouse(MOUSE_BLANK as u16, 0, 0)
    }

    pub(crate) fn fn_cross_mouse(&mut self, _a: u32, _b: u32, _c: u32) -> bool {
        if self.vars[OBJECT_HELD as usize] != 0 {
            self.fn_open_close_hand(false);
        } else {
            self.sprite_mouse(MOUSE_CROSS as u16, 4, 4);
        }
        true
    }

    fn fn_get_to(&mut self, target_place: u32, mode: u32) -> bool {
        let c = self.compact;
        self.s(c, f::UP_FLAG, mode as u16); // save mode for action script
        let m = self.g(c, f::MODE) + 4;
        self.s(c, f::MODE, m);
        let Some(place) = self.fetch(self.g(c, f::PLACE) as u32) else {
            log::warn!("can't find the compact's getToTable: place compact is NULL");
            return false;
        };
        let Some(mut table) = self.fetch(self.g(place, f::GET_TO_TABLE_ID) as u32) else {
            log::warn!("place compact's getToTable is NULL");
            return false;
        };
        while self.w(table) as u32 != target_place {
            table += 2;
        }
        let scr = self.w(table + 1);
        self.set_sub(c, m, scr);
        self.set_sub(c, m + 2, 0);
        false
    }

    fn fn_set_to_stand(&mut self) -> bool {
        let c = self.compact;
        self.s(c, f::MOOD, 1); // stood still
        let off = C_STAND_UP as u16 + self.g(c, f::MEGA_SET) + self.g(c, f::DIR) * 4;
        let stand = self.w(self.elem(c, off));
        self.s(c, f::GRAFIX_PROG_ID, stand);
        self.s(c, f::GRAFIX_PROG_POS, 0);
        let list = self.grafix_ptr(c).expect("stand list");
        self.s(c, f::OFFSET, self.w(list)); // frames offset
        self.s(c, f::LOGIC, L_SIMPLE_MOD as u16);
        let p = self.g(c, f::GRAFIX_PROG_POS) + 1;
        self.s(c, f::GRAFIX_PROG_POS, p);
        self.simple_anim();
        false
    }

    fn fn_turn_to(&mut self, dir: u32) -> bool {
        let c = self.compact;
        let cur = self.g(c, f::DIR);
        self.s(c, f::DIR, (dir & 0xFFFF) as u16);
        let tt = self.turn_table(c, cur);
        let turn = self.w(tt + dir as usize);
        if turn == 0 {
            return true; // keep going
        }
        self.s(c, f::TURN_PROG_ID, turn);
        self.s(c, f::TURN_PROG_POS, 0);
        self.s(c, f::LOGIC, L_TURNING as u16);
        self.turn();
        false
    }

    fn fn_arrived(&mut self, script_var: u32) -> bool {
        let c = self.compact;
        self.s(c, f::LEAVING, (script_var & 0xFFFF) as u16);
        let v = &mut self.vars[script_var as usize / 4];
        *v = v.wrapping_add(1);
        true
    }

    fn fn_leaving(&mut self) -> bool {
        let c = self.compact;
        self.s(c, f::AT_WATCH, 0);
        let leaving = self.g(c, f::LEAVING);
        if leaving != 0 {
            let v = &mut self.vars[leaving as usize / 4];
            *v = v.wrapping_sub(1);
            self.s(c, f::LEAVING, 0); // only once
        }
        true
    }

    fn fn_set_alternate(&mut self, scr: u32) -> bool {
        let c = self.compact;
        self.s(c, f::ALT, (scr & 0xFFFF) as u16);
        self.s(c, f::LOGIC, L_ALT as u16);
        false
    }

    fn fn_alt_set_alternate(&mut self, target: u32, scr: u32) -> bool {
        let t = self.cpt_start(target);
        self.s(t, f::ALT, (scr & 0xFFFF) as u16);
        self.s(t, f::LOGIC, L_ALT as u16);
        false
    }

    fn fn_kill_id(&mut self, id: u32) -> bool {
        if id != 0 {
            let t = self.cpt_start(id);
            if self.g(t, f::STATUS) & (1 << 7) != 0 {
                self.remove_object_from_walk(t);
            }
            self.s(t, f::STATUS, 0);
        }
        true
    }

    pub(crate) fn fn_no_human(&mut self, _a: u32, _b: u32, _c: u32) -> bool {
        if self.vars[MOUSE_STOP as usize] == 0 {
            self.vars[MOUSE_STATUS as usize] &= 1;
            self.run_get_off();
            self.fn_blank_mouse();
        }
        true
    }

    fn fn_speak_me(&mut self, target: u32, mesg: u32, anim: u32) -> bool {
        // WORKAROUND for #4230: while Mrs. Piermont is on the phone in her
        // flat, ignore her lines on other screens; they have no speech
        // files and would cut Foster's speech short
        if target == ID_DANIELLE && anim == 0x9B && self.vars[SCREEN as usize] != 38 {
            return false;
        }
        let t = self.cpt_start(target);
        self.std_speak(t, mesg, anim, 0);
        false
    }

    fn fn_speak_me_dir(&mut self, target: u32, mesg: u32, anim: u32) -> bool {
        // the player; the anim follows c_dir (2 sizes, large and small)
        let anim = anim + ((self.g(self.compact, f::DIR) as u32) << 1);
        self.fn_speak_me(target, mesg, anim)
    }

    fn fn_speak_wait(&mut self, id: u32, mesg: u32, anim: u32) -> bool {
        // a non-player mega speaks; the player waits for it to finish
        let c = self.compact;
        self.s(c, f::FLAG, (id & 0xFFFF) as u16);
        self.s(c, f::LOGIC, L_LISTEN as u16);
        self.fn_speak_me(id, mesg, anim)
    }

    fn fn_speak_wait_dir(&mut self, a: u32, b: u32, c: u32) -> bool {
        // a is the id to speak (not us), b the text, c the base of a mini
        // table within anim_talk_table
        let cp = self.compact;
        self.s(cp, f::FLAG, a as u16);
        self.s(cp, f::LOGIC, L_LISTEN as u16);
        let speaker = self.cpt_start(a);
        if c != 0 {
            let dir = (self.g(speaker, f::DIR) as u32) << 1;
            self.std_speak(speaker, b, c + dir, dir);
        } else {
            self.std_speak(speaker, b, c, 0);
        }
        false
    }

    fn fn_chooser(&mut self) -> bool {
        // set up the text questions to be clicked on, from TEXT1 until 0
        self.sys.system_flags |= SF_CHOOSING; // no save/restore while choosing
        self.vars[THE_CHOSEN_ONE as usize] = 0;
        let mut p = TEXT1 as usize;
        let mut ycood = TOP_LEFT_Y as u16;
        while self.vars[p] != 0 {
            let text_num = self.vars[p];
            p += 1;
            let low = self.low_text_manager(text_num, GAME_SCREEN_WIDTH as u16, 0, 241, Align::Left);

            // stipple the text
            let item = self.text_item_of(low.compact_num);
            let data = self.items[item].as_mut().expect("text sprite");
            let h = Header::parse(data);
            let (width, height) = (h.width as usize, h.height as usize);
            let px = &mut data[22..];
            let size = width * height;
            let mut index = 0usize;
            while index < size {
                if index % width <= 1 {
                    index ^= 1;
                }
                if index < px.len() && px[index] == 0 {
                    px[index] = 1;
                }
                index += 2;
            }

            let t = self.cpt_start(low.compact_num as u32);
            self.s(t, f::GET_TO_FLAG, text_num as u16);
            self.s(t, f::DOWN_FLAG, self.vars[p] as u16); // animation number
            p += 1;
            let st = self.g(t, f::STATUS) | ST_MOUSE as u16;
            self.s(t, f::STATUS, st);
            self.s(t, f::XCOOD, TOP_LEFT_X as u16);
            self.s(t, f::YCOOD, ycood);
            ycood += height as u16;
        }
        if p == TEXT1 as usize {
            return true;
        }
        self.mouse.set_text_chooser_mode();
        let c = self.compact;
        self.s(c, f::LOGIC, L_CHOOSE as u16); // player frozen until choice made
        self.fn_add_human();
        false
    }

    fn fn_highlight(&mut self, item: u32, pen: u32) -> bool {
        let pen = ((pen.wrapping_sub(11)) ^ 1).wrapping_add(241);
        let t = self.cpt_start(item);
        let spr = self.g(t, f::FLAG) as usize;
        self.change_text_sprite_color(spr, pen as u8);
        true
    }

    fn fn_text_kill(&mut self) -> bool {
        // kill text items that are mouse detectable
        for id in FIRST_TEXT_COMPACT..FIRST_TEXT_COMPACT + 10 {
            let t = self.cpt_start(id);
            if self.g(t, f::STATUS) & (1 << 4) != 0 {
                self.s(t, f::STATUS, 0);
            }
        }
        true
    }

    fn fn_we_wait(&mut self, id: u32) -> bool {
        let c = self.compact;
        self.s(c, f::WAITING_FOR, id as u16);
        self.stop_and_wait();
        true // not sure about this (ScummVM's comment)
    }

    fn fn_check_request(&mut self) -> bool {
        let c = self.compact;
        let req = self.g(c, f::REQUEST);
        if req == 0 {
            return true;
        }
        self.s(c, f::MODE, C_ACTION_MODE as u16);
        self.s(c, f::ACTION_SUB, req);
        self.s(c, f::ACTION_SUB_OFF, 0);
        self.s(c, f::REQUEST, 0);
        false
    }

    /// Initialise the top menu bar; first_object is o0 for the game menu,
    /// k0 for the linc.
    fn fn_start_menu(&mut self, first_object: u32) -> bool {
        let first = (first_object / 4) as usize;
        let screen = (self.vars[SCREEN as usize] & 0xFFFF) as u16;
        let live = (ST_MOUSE + ST_FOREGROUND + ST_LOGIC + ST_RECREATE) as u16;

        // (1) the two arrows
        for id in [47u32, 48] {
            let t = self.cpt_start(id);
            self.s(t, f::STATUS, live);
            self.s(t, f::SCREEN, screen);
        }

        // (2) objects from non-zero inventory variables
        let mut menu_len = 0usize;
        for i in first..first + self.object_list.len() {
            if self.vars[i] != 0 {
                self.object_list[menu_len] = self.vars[i];
                menu_len += 1;
            }
        }
        self.vars[MENU_LENGTH as usize] = menu_len as u32;

        // (3) top up with blank objects, for a minimum display of 11
        let mut blank = 51u32;
        for i in menu_len..11 {
            self.object_list[i] = blank;
            blank += 1;
        }

        // (4) kill all 20 objects so scrolled-off icons don't remain
        for i in 0..self.object_list.len() {
            if self.object_list[i] == 0 {
                break;
            }
            let t = self.cpt_start(self.object_list[i]);
            self.s(t, f::STATUS, ST_LOGIC as u16);
        }

        // (5) which object to start the display from
        if menu_len < 11 {
            self.vars[SCROLL_OFFSET as usize] = 0;
        } else if (menu_len as u32) < self.vars[SCROLL_OFFSET as usize] + 11 {
            self.vars[SCROLL_OFFSET as usize] = menu_len as u32 - 11;
        }

        // (6) and the 11 objects on screen
        let mut x = (TOP_LEFT_X + 28) as u16;
        for i in 0..11 {
            let id = self.object_list[self.vars[SCROLL_OFFSET as usize] as usize + i];
            let t = self.cpt_start(id);
            self.s(t, f::STATUS, live);
            self.s(t, f::SCREEN, screen);
            self.s(t, f::XCOOD, x);
            x += 24;
            let y = if self.vars[MENU as usize] == 2 { 136 } else { 112 };
            self.s(t, f::YCOOD, y);
        }
        true
    }

    fn fn_face_id(&mut self, other: u32) -> bool {
        // the direction to turn to face another id, in c_get_to_flag
        let c = self.compact;
        let o = self.cpt_start(other);
        let mut x = self.g(c, f::XCOOD).wrapping_sub(self.g(o, f::XCOOD)) as i16;
        if x < 0 {
            x = x.wrapping_neg();
            self.s(c, f::GET_TO_FLAG, 3);
        } else {
            self.s(c, f::GET_TO_FLAG, 2);
        }
        // the true bottom of the sprite: the bottom of its mouse area
        let bottom = self
            .g(o, f::YCOOD)
            .wrapping_add(self.g(o, f::MOUSE_REL_Y))
            .wrapping_add(self.g(o, f::MOUSE_SIZE_Y));
        let mut y = self.g(c, f::YCOOD).wrapping_sub(bottom) as i16;
        if y < 0 {
            // it's below
            y = y.wrapping_neg();
            if y >= x {
                self.s(c, f::GET_TO_FLAG, 1);
            }
        } else if y >= x {
            // it's above
            self.s(c, f::GET_TO_FLAG, 0);
        }
        true
    }

    /// fnForeground, fnBackground, fnNewBackground, fnSort,
    /// fnNoSpriteEngine, fnNoSpritesA6.
    fn set_sprite_kind(&mut self, id: Option<u32>, kind: u32) -> bool {
        let t = match id {
            Some(id) => self.cpt_start(id),
            None => self.compact,
        };
        let st = (self.g(t, f::STATUS) & 0xFFF8) | kind as u16;
        self.s(t, f::STATUS, st);
        true
    }

    fn fn_reset_id(&mut self, id: u32, reset_block: u32) -> bool {
        // restart a mega: a smaller mega turning larger, changing rooms ...
        let (Some(t), Some(mut rst)) = (self.fetch(id), self.fetch(reset_block)) else {
            log::warn!("fnResetId({id}, {reset_block}): compact missing");
            return true;
        };
        loop {
            let off = self.w(rst);
            rst += 1;
            if off == 0xFFFF {
                break;
            }
            let v = self.w(rst);
            rst += 1;
            let at = self.elem(t, off);
            self.sw(at, v);
        }
        true
    }

    fn fn_run_anim_mod(&mut self, anim: u32) -> bool {
        let c = self.compact;
        self.s(c, f::GRAFIX_PROG_ID, anim as u16);
        self.s(c, f::GRAFIX_PROG_POS, 0);
        let gp = self.grafix_ptr(c).expect("anim");
        self.s(c, f::OFFSET, self.w(gp));
        let p = self.g(c, f::GRAFIX_PROG_POS) + 1;
        self.s(c, f::GRAFIX_PROG_POS, p);
        self.s(c, f::LOGIC, L_MOD_ANIMATE as u16);
        self.anim();
        false
    }

    /// fnSimpleMod (L_SIMPLE_MOD) and fnRunFrames (L_FRAMES).
    fn fn_simple_mod(&mut self, seq: u32, logic: u32) -> bool {
        let c = self.compact;
        self.s(c, f::GRAFIX_PROG_ID, seq as u16);
        self.s(c, f::GRAFIX_PROG_POS, 0);
        self.s(c, f::LOGIC, logic as u16);
        let gp = self.grafix_ptr(c).expect("anim");
        self.s(c, f::OFFSET, self.w(gp));
        let p = self.g(c, f::GRAFIX_PROG_POS) + 1;
        self.s(c, f::GRAFIX_PROG_POS, p);
        self.simple_anim();
        false
    }

    fn fn_move_items(&mut self, list_no: u32, screen_no: u32) -> bool {
        // move a list of ids to another screen
        let ml = self.cpt_start(CPT_MOVE_LIST);
        let mut p = self.cpt_start(self.w(ml + list_no as usize) as u32);
        for _ in 0..2 {
            let id = self.w(p);
            if id == 0 {
                return true;
            }
            p += 1;
            let t = self.cpt_start(id as u32);
            self.s(t, f::SCREEN, screen_no as u16);
        }
        true
    }

    fn fn_test_list(&mut self, id: u32, x: u32, y: u32) -> bool {
        self.vars[RESULT as usize] = 0; // assume fail
        let mut l = self.cpt_start(id);
        while self.w(l) != 0 {
            let r = |i: usize| self.w(l + i) as u32;
            if x >= r(0) && x < r(1) && y >= r(2) && y < r(3) {
                self.vars[RESULT as usize] = r(4);
            }
            l += 5;
        }
        true
    }

    fn fn_eyeball(&mut self, id: u32) -> bool {
        // frame number pointing at Foster, from the given table
        let table = self.cpt_start(id);
        let foster = self.cpt_start(ID_BLUE_FOSTER);
        let x = (self.g(foster, f::XCOOD) as u32).wrapping_sub(168) >> 3; // 168 < x < 416
        let y = (self.g(foster, f::YCOOD) as u32).wrapping_sub(256) << 2; // 256 < y < 296
        let idx = x.wrapping_add(y) as usize;
        self.vars[RESULT as usize] = self.w(table + idx) as u32 + S91;
        true
    }

    pub(crate) fn fn_leave_section(&mut self, section: u32, _b: u32, _c: u32) -> bool {
        if self.disk.is_demo() {
            self.quit = true;
        }
        if section == 5 {
            // linc section: different mouse icons
            self.replace_mouse_cursors(60301);
        }
        true
    }

    pub(crate) fn fn_enter_section(&mut self, section: u32, _b: u32, _c: u32) -> bool {
        if self.disk.is_demo() && section > 2 {
            self.quit = true;
        }
        self.vars[CUR_SECTION as usize] = section;
        self.sys.current_music = 0;
        if section == 5 {
            self.replace_mouse_cursors(60302);
        }
        if section != self.current_section || self.sys.system_flags & SF_GAME_RESTORED != 0 {
            self.current_section = section;
            let s = (section + 1) as u8;
            self.audio_load_section(s);
            self.load_grids();
            self.sys.system_flags &= !SF_GAME_RESTORED;
        }
        true
    }

    fn fn_print_credit(&mut self, a: u32, c: u32) -> bool {
        let t = self.low_text_manager(a, 240, 0, 248, Align::Center);
        let cp = self.cpt_start(t.compact_num as u32);
        self.s(cp, f::XCOOD, 168);
        if a == 558 && c == 215 {
            self.s(cp, f::YCOOD, 211);
        } else {
            self.s(cp, f::YCOOD, c as u16);
        }
        self.vars[RESULT as usize] = t.compact_num as u32;
        true
    }

    fn fn_look_at(&mut self, a: u32, c: u32) -> bool {
        let t = self.low_text_manager(a, 240, 0, 248, Align::Center);
        let cp = self.cpt_start(t.compact_num as u32);
        self.s(cp, f::XCOOD, 168);
        self.s(cp, f::YCOOD, c as u16);

        self.recreate();
        self.sprite_engine();
        self.flip(true);

        self.fn_no_human(0, 0, 0);
        self.sys.system_flags |= SF_MOUSE_LOCKED;
        self.wait_mouse_not_pressed(800);
        self.sys.system_flags &= !SF_MOUSE_LOCKED;
        self.fn_add_human();
        self.s(cp, f::STATUS, 0);
        true
    }

    fn fn_linc_text_module(&mut self, text_pos: u32, text_no: u32, button_action: u32) -> bool {
        if button_action & 0x8000 != 0 {
            for v in LINC_DIGIT_0..=LINC_DIGIT_9 {
                self.vars[v as usize] = 0;
            }
        }
        let button_action = button_action & 0x7FFF;
        if button_action < 10 {
            self.vars[(LINC_DIGIT_0 + button_action) as usize] = text_no;
        }
        let t = self.low_text_manager(text_no, 220, 0, 215, Align::Left);
        let cp = self.cpt_start(t.compact_num as u32);
        if text_pos < 20 {
            // line number (for text)
            self.s(cp, f::XCOOD, 152);
            self.s(cp, f::YCOOD, (text_pos * 13 + 170) as u16);
        } else if text_pos > 20 {
            // x coordinate (for numbers)
            self.s(cp, f::XCOOD, text_pos as u16);
            self.s(cp, f::YCOOD, 214);
        } else {
            log::warn!("fnLincTextModule: textPos == 20");
        }
        self.s(cp, f::GET_TO_FLAG, text_no as u16);
        true
    }

    pub(crate) fn fn_start_fx(&mut self, sound: u32, b: u32, _c: u32) -> bool {
        self.audio_start_fx(sound, (b & 1) as u8);
        true
    }

    /// Logic::stdSpeak: make `target` talk text `text_num` with talk
    /// animation `anim_num`.
    pub(crate) fn std_speak(&mut self, target: usize, text_num: u32, anim_num: u32, _base: u32) {
        let mut anim = anim_num + (self.g(target, f::MEGA_SET) as u32 / NEXT_MEGA_SET);
        anim &= 0xFF;
        let talk_table = self.cpt_start(CPT_TALK_TABLE_LIST);
        self.s(target, f::GRAFIX_PROG_ID, self.w(talk_table + anim as usize));
        self.s(target, f::GRAFIX_PROG_POS, 0);
        if let Some(gp) = self.grafix_ptr(target) {
            self.s(target, f::OFFSET, self.w(gp));
            self.s(target, f::GET_TO_FLAG, self.w(gp + 1));
            let p = self.g(target, f::GRAFIX_PROG_POS) + 2;
            self.s(target, f::GRAFIX_PROG_POS, p);
        } else {
            self.s(target, f::GRAFIX_PROG_ID, 0);
        }

        let speech_found = self.disk.is_cd() && self.audio_start_speech(text_num as u16);

        if self.sys.system_flags & SF_ALLOW_TEXT != 0 || !speech_found {
            // the text sprite, if the player wants subtitles or there is
            // no speech file
            let color = self.g(target, f::SP_COLOR) as u8;
            let info = self.low_text_manager(text_num, FIXED_TEXT_WIDTH as u16, 0, color, Align::Center);
            let tc = self.cpt_start(info.compact_num as u32);
            self.s(target, f::SP_TEXT_ID, info.compact_num); // so we know what text to kill
            self.s(tc, f::SCREEN, self.g(target, f::SCREEN));

            if self.vars[SCREEN as usize] == self.g(target, f::SCREEN) as u32 {
                // talking on screen: centre the text over the talker
                let frame = self.g(target, f::FRAME);
                let th = Header::parse(self.item((frame >> 6) as u32).expect("talker sprite"));
                let text_h = info.height;
                let mut x = self.g(target, f::XCOOD).wrapping_add(th.offset_x as u16);
                let width = th.width >> 1;
                x = x.wrapping_add(width).wrapping_sub(FIXED_TEXT_WIDTH as u16 / 2);
                if x < TOP_LEFT_X as u16 {
                    x = TOP_LEFT_X as u16;
                }
                let right = x.wrapping_add(FIXED_TEXT_WIDTH as u16);
                if (TOP_LEFT_X + FULL_SCREEN_WIDTH) as u16 <= right {
                    x = (TOP_LEFT_X + FULL_SCREEN_WIDTH - FIXED_TEXT_WIDTH) as u16;
                }
                self.s(tc, f::XCOOD, x);
                let mut y = self
                    .g(target, f::YCOOD)
                    .wrapping_add(th.offset_y as u16)
                    .wrapping_sub(6)
                    .wrapping_sub(text_h);
                if y < TOP_LEFT_Y as u16 {
                    y = TOP_LEFT_Y as u16;
                }
                self.s(tc, f::YCOOD, y);
            } else {
                // talking off screen
                self.s(target, f::SP_TEXT_ID, 0); // no text was made to kill
                self.s(tc, f::STATUS, 0); // don't display it
            }
            // with a speech file the VOC timing rules, plus a 10 cycle pause
            if speech_found {
                self.s(target, f::SP_TIME, 10);
            } else {
                self.s(target, f::SP_TIME, (self.text.num_letters + 5) as u16);
            }
        } else {
            self.s(target, f::SP_TIME, 10);
            self.s(target, f::SP_TEXT_ID, 0);
        }
        self.s(target, f::LOGIC, L_TALK as u16);
    }
}
