//! The object logic and the script interpreter. Port of ScummVM's
//! engines/sky/logic.cpp (the logic modes and Logic::script); the mcode
//! functions the scripts call are in mcode.rs.

use super::defs::*;
use super::sky::{f, Sky, NUM_SKY_SCRIPTVARS};

const FORWARD_LIST_1B: [u32; 63] = [
    JOBS_SPEECH, JOBS_S4, JOBS_ALARMED, JOEY_RECYCLE, SHOUT_SSS, JOEY_MISSION, TRANS_MISSION,
    SLOT_MISSION, CORNER_MISSION, JOEY_LOGIC, GORDON_SPEECH, JOEY_BUTTON_MISSION, LOB_DAD_SPEECH,
    LOB_SON_SPEECH, GUARD_SPEECH, MANTRACH_SPEECH, WRECK_SPEECH, ANITA_SPEECH, LAMB_FACTORY,
    FORE_SPEECH, JOEY_42_MISS, JOEY_JUNCTION_MISS, WELDER_MISSION, JOEY_WELD_MISSION, RADMAN_SPEECH,
    LINK_7_29, LINK_29_7, LAMB_TO_3, LAMB_TO_2, BURKE_SPEECH, BURKE_1, BURKE_2, DR_BURKE_1,
    JASON_SPEECH, JOEY_BELLEVUE, ANCHOR_SPEECH, ANCHOR_MISSION, JOEY_PC_MISSION, HOOK_MISSION,
    TREVOR_SPEECH, JOEY_FACTORY, HELGA_SPEECH, JOEY_HELGA_MISSION, GALL_BELLEVUE, GLASS_MISSION,
    LAMB_FACT_RETURN, LAMB_LEAVE_GARDEN, LAMB_START_29, LAMB_BELLEVUE, CABLE_MISSION, FOSTER_TOUR,
    LAMB_TOUR, FOREMAN_LOGIC, LAMB_LEAVE_FACTORY, LAMB_BELL_LOGIC, LAMB_FACT_2, START90, 0, 0,
    LINK_28_31, LINK_31_28, EXIT_LINC, DEATH_SCRIPT,
];

const FORWARD_LIST_1B_288: [u32; 63] = [
    JOBS_SPEECH, JOBS_S4, JOBS_ALARMED, JOEY_RECYCLE, SHOUT_SSS, JOEY_MISSION, TRANS_MISSION,
    SLOT_MISSION, CORNER_MISSION, JOEY_LOGIC, GORDON_SPEECH, JOEY_BUTTON_MISSION, LOB_DAD_SPEECH,
    LOB_SON_SPEECH, GUARD_SPEECH, 0x68, WRECK_SPEECH, ANITA_SPEECH, LAMB_FACTORY, FORE_SPEECH,
    JOEY_42_MISS, JOEY_JUNCTION_MISS, WELDER_MISSION, JOEY_WELD_MISSION, RADMAN_SPEECH, LINK_7_29,
    LINK_29_7, LAMB_TO_3, LAMB_TO_2, 0x3147, 0x3100, 0x3101, 0x3102, 0x3148, 0x3149, 0x314A, 0x30C5,
    0x30C6, 0x30CB, 0x314B, JOEY_FACTORY, 0x314C, 0x30E2, 0x314D, 0x310C, LAMB_FACT_RETURN, 0x3139,
    0x313A, 0x004F, CABLE_MISSION, FOSTER_TOUR, LAMB_TOUR, FOREMAN_LOGIC, LAMB_LEAVE_FACTORY, 0x3138,
    LAMB_FACT_2, 0x004D, 0, 0, LINK_28_31, LINK_31_28, 0x004E, DEATH_SCRIPT,
];

const FORWARD_LIST_2B: [u32; 7] =
    [STD_ON, STD_EXIT_LEFT_ON, STD_EXIT_RIGHT_ON, ADVISOR_188, SHOUT_ACTION, MEGA_CLICK, MEGA_ACTION];

const FORWARD_LIST_3B: [u32; 21] = [
    DANI_SPEECH, DANIELLE_GO_HOME, SPUNKY_GO_HOME, HENRI_SPEECH, BUZZER_SPEECH, FOSTER_VISIT_DANI,
    DANIELLE_LOGIC, JUKEBOX_SPEECH, VINCENT_SPEECH, EDDIE_SPEECH, BLUNT_SPEECH, DANI_ANSWER_PHONE,
    SPUNKY_SEE_VIDEO, SPUNKY_BARK_AT_FOSTER, SPUNKY_SMELLS_FOOD, BARRY_SPEECH, COLSTON_SPEECH,
    GALL_SPEECH, BABS_SPEECH, CHUTNEY_SPEECH, FOSTER_ENTER_COURT,
];

const FORWARD_LIST_4B: [u32; 14] = [
    WALTER_SPEECH, JOEY_MEDIC, JOEY_MED_LOGIC, JOEY_MED_MISSION72, KEN_LOGIC, KEN_SPEECH,
    KEN_MISSION_HAND, SC70_IRIS_OPENED, SC70_IRIS_CLOSED, FOSTER_ENTER_BOARDROOM, BORED_ROOM,
    FOSTER_ENTER_NEW_BOARDROOM, HOBS_END, SC82_JOBS_SSS,
];

const FORWARD_LIST_5B: [u32; 7] =
    [SET_UP_INFO_WINDOW, SLAB_ON, UP_MOUSE, DOWN_MOUSE, LEFT_MOUSE, RIGHT_MOUSE, DISCONNECT_FOSTER];

/// Ids whose speech the player may click through.
const CLICK_TABLE: [u32; 46] = [
    ID_FOSTER, ID_JOEY, ID_JOBS, ID_LAMB, ID_ANITA, ID_SON, ID_DAD, ID_MONITOR, ID_SHADES, MINI_SS,
    FULL_SS, ID_FOREMAN, ID_RADMAN, ID_GALLAGER_BEL, ID_BURKE, ID_BODY, ID_HOLO, ID_TREVOR,
    ID_ANCHOR, ID_WRECK_GUARD, ID_SKORL_GUARD,
    // base level
    ID_SC30_HENRI, ID_SC31_GUARD, ID_SC32_VINCENT, ID_SC32_GARDENER, ID_SC32_BUZZER, ID_SC36_BABS,
    ID_SC36_BARMAN, ID_SC36_COLSTON, ID_SC36_GALLAGHER, ID_SC36_JUKEBOX, ID_DANIELLE, ID_SC42_JUDGE,
    ID_SC42_CLERK, ID_SC42_PROSECUTION, ID_SC42_JOBSWORTH,
    // underworld
    ID_MEDI, ID_WITNESS, ID_GALLAGHER, ID_KEN, ID_SC76_ANDROID_2, ID_SC76_ANDROID_3, ID_SC81_FATHER,
    ID_SC82_JOBSWORTH,
    // linc world
    ID_HOLOGRAM_B, 12289,
];

impl Sky {
    pub(crate) fn logic_init(&mut self) {
        self.object_list = [0; 30];
        self.modules = Default::default();
        self.stack.clear();
        self.current_section = 0xFF; // force music & sound reload
        self.init_script_variables();
    }

    pub(crate) fn init_script_variables(&mut self) {
        let v = &mut self.vars;
        *v = [0; NUM_SKY_SCRIPTVARS];
        v[LOGIC_LIST_NO as usize] = 141;
        v[LAMB_GREET as usize] = 62;
        v[JOEY_SECTION as usize] = 1;
        v[LAMB_SECTION as usize] = 2;
        v[S15_FLOOR as usize] = 8371;
        v[GUARDIAN_THERE as usize] = 1;
        v[DOOR_67_68_FLAG as usize] = 1;
        v[SC70_IRIS_FLAG as usize] = 3;
        v[DOOR_73_75_FLAG as usize] = 1;
        v[SC76_CABINET1_FLAG as usize] = 1;
        v[SC76_CABINET2_FLAG as usize] = 1;
        v[SC76_CABINET3_FLAG as usize] = 1;
        v[DOOR_77_78_FLAG as usize] = 1;
        v[SC80_EXIT_FLAG as usize] = 1;
        v[SC31_LIFT_FLAG as usize] = 1;
        v[SC32_LIFT_FLAG as usize] = 1;
        v[SC33_SHED_DOOR_FLAG as usize] = 1;
        v[BAND_PLAYING as usize] = 1;
        v[COLSTON_AT_TABLE as usize] = 1;
        v[SC36_NEXT_DEALER as usize] = 16731;
        v[SC36_DOOR_FLAG as usize] = 1;
        v[SC37_DOOR_FLAG as usize] = 2;
        v[SC40_LOCKER_1_FLAG as usize] = 1;
        v[SC40_LOCKER_2_FLAG as usize] = 1;
        v[SC40_LOCKER_3_FLAG as usize] = 1;
        v[SC40_LOCKER_4_FLAG as usize] = 1;
        v[SC40_LOCKER_5_FLAG as usize] = 1;
        let list1 = if self.sys.game_version == 288 { &FORWARD_LIST_1B_288 } else { &FORWARD_LIST_1B };
        v[352..352 + list1.len()].copy_from_slice(list1);
        v[656..656 + FORWARD_LIST_2B.len()].copy_from_slice(&FORWARD_LIST_2B);
        v[721..721 + FORWARD_LIST_3B.len()].copy_from_slice(&FORWARD_LIST_3B);
        v[663..663 + FORWARD_LIST_4B.len()].copy_from_slice(&FORWARD_LIST_4B);
        v[505..505 + FORWARD_LIST_5B.len()].copy_from_slice(&FORWARD_LIST_5B);
    }

    pub fn init_screen0(&mut self) {
        self.fn_enter_section(0, 0, 0);
        self.audio_start_music(2);
        self.sys.current_music = 2;
    }

    /// Logic::parseSaveData: the script variables part of a restore.
    pub fn parse_save_vars(&mut self, data: &[u32]) {
        if !self.disk.is_demo() {
            self.fn_leave_section(self.vars[CUR_SECTION as usize], 0, 0);
        }
        self.vars.copy_from_slice(&data[..NUM_SKY_SCRIPTVARS]);
        self.fn_enter_section(self.vars[CUR_SECTION as usize], 0, 0);
    }

    fn check_protection(&mut self) -> bool {
        if self.vars[ENTER_DIGITS as usize] != 0 {
            if self.vars[CONSOLE_TYPE as usize] == 5 {
                // reactor code
                self.vars[FS_COMMAND as usize] = 240;
            } else {
                // copy protection
                self.vars[FS_COMMAND as usize] = 337;
            }
            self.vars[ENTER_DIGITS as usize] = 0;
            true
        } else {
            false
        }
    }

    /// Logic::engine: one game cycle of every compact in the logic list.
    pub fn logic_engine(&mut self) {
        loop {
            let mut list = self.cpt_start(self.vars[LOGIC_LIST_NO as usize]);
            loop {
                let id = self.w(list);
                list += 1;
                if id == 0 {
                    break;
                }
                if id == 0xFFFF {
                    // change logic data address
                    list = self.cpt_start(self.w(list) as u32);
                    continue;
                }
                self.vars[CUR_ID as usize] = id as u32;
                let c = self.cpt_start(id as u32);
                self.compact = c;

                // does the id wish to be processed?
                if self.g(c, f::STATUS) & (1 << 6) == 0 {
                    continue;
                }
                if self.g(c, f::STATUS) & (1 << 7) != 0 {
                    self.remove_object_from_walk(c);
                }

                let logic = self.g(c, f::LOGIC);
                self.run_logic(logic);

                if self.g(c, f::STATUS) & (1 << 7) != 0 {
                    self.object_to_walk(c);
                }
                // a sync sent to the compact is available for one cycle only
                self.s(c, f::SYNC, 0);
                if self.quit {
                    return;
                }
            }
            // run again only when the script asked for a copy protection code,
            // so its screen does not flash up for one cycle
            if !self.check_protection() {
                break;
            }
        }
    }

    fn run_logic(&mut self, logic: u16) {
        match logic as u32 {
            0 => {}
            L_SCRIPT => self.logic_script(),
            L_AR => self.auto_route_logic(),
            L_AR_ANIM => self.ar_anim(),
            L_AR_TURNING => self.ar_turn(),
            L_ALT => self.alt(),
            L_MOD_ANIMATE => self.anim(),
            L_TURNING => self.turn(),
            L_CURSOR => self.cursor_logic(),
            L_TALK => self.talk(),
            L_LISTEN => self.listen(),
            L_STOPPED => self.stopped(),
            L_CHOOSE => self.choose(),
            L_FRAMES => self.frames(),
            L_PAUSE => self.pause_logic(),
            L_WAIT_SYNC => self.wait_sync(),
            L_SIMPLE_MOD => self.simple_anim(),
            n => panic!("unknown logic mode {n}"),
        }
    }

    /// Runs the current compact's script until it finishes or drops out.
    pub(crate) fn logic_script(&mut self) {
        let c = self.compact;
        loop {
            let mode = self.g(c, f::MODE);
            let script_no = self.get_sub(c, mode);
            let offset = self.get_sub(c, mode + 2);
            let offset = self.script(script_no, offset);
            self.set_sub(c, mode + 2, offset);
            if offset == 0 {
                // script finished, drop back a level
                let m = self.g(c, f::MODE).wrapping_sub(4);
                self.s(c, f::MODE, m);
            } else if self.g(c, f::MODE) == mode {
                return;
            }
            if self.quit {
                return;
            }
        }
    }

    fn auto_route_logic(&mut self) {
        let c = self.compact;
        let mut down = self.auto_route(c);
        if down == 2
            && self.cpt_is_id(c, CPT_JOEY)
            && self.g(c, f::MODE) == 0
            && self.g(c, f::BASE_SUB) as u32 == JOEY_OUT_OF_LIFT
        {
            // workaround for script bug #1823
            down = 0;
        }
        self.s(c, f::DOWN_FLAG, down);
        if down != 1 {
            // route ok
            let scratch = self.g(c, f::ANIM_SCRATCH_ID);
            self.s(c, f::GRAFIX_PROG_ID, scratch);
            self.s(c, f::GRAFIX_PROG_POS, 0);
        }
        self.s(c, f::LOGIC, L_SCRIPT as u16);
        self.logic_script();
    }

    fn ar_anim(&mut self) {
        let c = self.compact;
        // only check collisions on character boundaries
        if self.g(c, f::XCOOD) & 7 != 0 || self.g(c, f::YCOOD) & 7 != 0 {
            self.main_anim();
            return;
        }
        // the first cycle of a re-route needs no collision check
        if self.g(c, f::WAITING_FOR) == 0xFFFF {
            self.main_anim();
            return;
        }
        let waiting = self.g(c, f::WAITING_FOR);
        if waiting != 0 {
            // we were told we hit someone; wait until we no longer collide
            let other = self.cpt_start(waiting as u32);
            if self.is_collision(other) {
                self.stop_and_wait();
                return;
            }
            self.s(c, f::WAITING_FOR, 0);
        }

        // our turn to check for collisions
        let mut list = self.cpt_start(self.vars[LOGIC_LIST_NO as usize]);
        loop {
            let id = self.w(list);
            list += 1;
            if id == 0 {
                break;
            }
            if id == 0xFFFF {
                list = self.cpt_start(self.w(list) as u32);
                continue;
            }
            if id == (self.vars[CUR_ID as usize] & 0xFFFF) as u16 {
                continue;
            }
            self.vars[HIT_ID as usize] = id as u32; // for any c_mini_bump
            let other = self.cpt_start(id as u32);
            if self.g(other, f::STATUS) & (1 << ST_COLLISION_BIT) == 0 {
                continue;
            }
            if self.g(other, f::SCREEN) != self.g(c, f::SCREEN) {
                continue;
            }
            if self.is_collision(other) {
                if self.g(other, f::LOGIC) as u32 != L_AR_ANIM {
                    // it is doing something else: restart our get-to script,
                    // and tell it to wait for us in case it starts moving
                    self.s(c, f::WAITING_FOR, 0xFFFF);
                    self.s(other, f::WAITING_FOR, (self.vars[CUR_ID as usize] & 0xFFFF) as u16);
                    let m = self.g(c, f::MODE);
                    self.set_sub(c, m + 2, 0);
                    self.s(c, f::LOGIC, L_SCRIPT as u16);
                    self.logic_script();
                    return;
                }
                let bump = self.g(c, f::MINI_BUMP);
                self.script(bump, 0);
                return;
            }
        }

        // no collisions; check for an interaction request (always an action script)
        let request = self.g(c, f::REQUEST);
        if request != 0 {
            self.s(c, f::MODE, C_ACTION_MODE as u16);
            self.s(c, f::ACTION_SUB, request);
            self.s(c, f::ACTION_SUB_OFF, 0);
            self.s(c, f::REQUEST, 0);
            self.s(c, f::LOGIC, L_SCRIPT as u16);
            self.logic_script();
            return;
        }

        // any flag set? if it changed, re-run the current (position
        // independent) get-to script
        let at_watch = self.g(c, f::AT_WATCH);
        if at_watch == 0 {
            self.main_anim();
            return;
        }
        if self.g(c, f::AT_WAS) as u32 == self.vars[at_watch as usize / 4] {
            self.main_anim();
            return;
        }
        let m = self.g(c, f::MODE);
        self.set_sub(c, m + 2, 0);
        self.s(c, f::LOGIC, L_SCRIPT as u16);
        self.logic_script();
    }

    fn main_anim(&mut self) {
        let c = self.compact;
        self.s(c, f::WAITING_FOR, 0); // clear possible zero-zero skip

        let mut seq = self.grafix_ptr(c).expect("route sequence");
        if self.w(seq) == 0 {
            // move to the next anim segment
            seq += 2;
            let p = self.g(c, f::GRAFIX_PROG_POS) + 2;
            self.s(c, f::GRAFIX_PROG_POS, p);
            if self.w(seq) == 0 {
                // end of route
                self.s(c, f::AR_ANIM_INDEX, 0);
                self.s(c, f::DOWN_FLAG, 0); // pass back ok to script
                self.s(c, f::LOGIC, L_SCRIPT as u16);
                self.logic_script();
                return;
            }
            self.s(c, f::AR_ANIM_INDEX, 0);
        }

        let mut dir;
        loop {
            dir = self.g(c, f::DIR);
            let want = self.w(seq + 1);
            if dir == want {
                break;
            }
            // set up turning
            self.s(c, f::DIR, want);
            let tt = self.turn_table(c, dir);
            let turn = self.w(tt + want as usize);
            if turn != 0 {
                self.s(c, f::TURN_PROG_ID, turn);
                self.s(c, f::TURN_PROG_POS, 0);
                self.s(c, f::LOGIC, L_AR_TURNING as u16);
                self.ar_turn();
                return;
            }
        }

        let ms = self.g(c, f::MEGA_SET);
        let anim_id = self.w(self.elem(c, (C_ANIM_UP as u16) + ms + dir * 4));
        let anim = self.cpt_start(anim_id as u32);

        let mut idx = self.g(c, f::AR_ANIM_INDEX) as usize;
        if self.w(anim + idx / 2) == 0 {
            idx = 0;
            self.s(c, f::AR_ANIM_INDEX, 0);
        }
        let ai = self.g(c, f::AR_ANIM_INDEX) + S_LENGTH as u16;
        self.s(c, f::AR_ANIM_INDEX, ai);

        let count = self.w(anim + (S_COUNT as usize + idx) / 2);
        let v = self.w(seq).wrapping_sub(count); // reduce the distance to travel
        self.sw(seq, v);
        let frame = self.w(anim + (S_FRAME as usize + idx) / 2);
        self.s(c, f::FRAME, frame);
        let x = self.g(c, f::XCOOD).wrapping_add(self.w(anim + (S_AR_X as usize + idx) / 2));
        self.s(c, f::XCOOD, x);
        let y = self.g(c, f::YCOOD).wrapping_add(self.w(anim + (S_AR_Y as usize + idx) / 2));
        self.s(c, f::YCOOD, y);
    }

    pub(crate) fn ar_turn(&mut self) {
        let c = self.compact;
        let td = self.cpt_start(self.g(c, f::TURN_PROG_ID) as u32) + self.g(c, f::TURN_PROG_POS) as usize;
        self.s(c, f::FRAME, self.w(td));
        let p = self.g(c, f::TURN_PROG_POS) + 1;
        self.s(c, f::TURN_PROG_POS, p);
        if self.w(td + 1) == 0 {
            // turn done, back to ar mode
            self.s(c, f::AR_ANIM_INDEX, 0);
            self.s(c, f::LOGIC, L_AR_ANIM as u16);
        }
    }

    fn alt(&mut self) {
        let c = self.compact;
        self.s(c, f::LOGIC, L_SCRIPT as u16);
        let mode = self.g(c, f::MODE);
        let alt = self.g(c, f::ALT);
        self.set_sub(c, mode, alt);
        self.set_sub(c, mode + 2, 0);
        self.logic_script();
    }

    /// Follow an animation sequence.
    pub(crate) fn anim(&mut self) {
        let c = self.compact;
        let mut gp = self.grafix_ptr(c).expect("anim program");
        while self.w(gp) != 0 {
            let p = self.g(c, f::GRAFIX_PROG_POS) + 3; // all types are 3 words
            self.s(c, f::GRAFIX_PROG_POS, p);
            let cmd = self.w(gp) as u32;
            if cmd == LF_START_FX {
                let sound = self.w(gp + 1) as u32;
                let volume = self.w(gp + 2) as u32;
                gp += 3;
                self.fn_start_fx(sound, 0, volume); // channel 0
            } else if cmd >= LF_START_FX {
                // do sync
                let target = self.cpt_start(self.w(gp + 1) as u32);
                let sync = self.w(gp + 2);
                gp += 3;
                self.s(target, f::SYNC, sync);
            } else {
                // coordinates and frame
                self.s(c, f::XCOOD, self.w(gp));
                self.s(c, f::YCOOD, self.w(gp + 1));
                let frame = self.w(gp + 2) | self.g(c, f::OFFSET);
                self.s(c, f::FRAME, frame);
                return;
            }
        }
        self.s(c, f::DOWN_FLAG, 0);
        self.s(c, f::LOGIC, L_SCRIPT as u16);
        self.logic_script();
    }

    pub(crate) fn turn(&mut self) {
        let c = self.compact;
        let td = self.cpt_start(self.g(c, f::TURN_PROG_ID) as u32) + self.g(c, f::TURN_PROG_POS) as usize;
        if self.w(td) != 0 {
            self.s(c, f::FRAME, self.w(td));
            let p = self.g(c, f::TURN_PROG_POS) + 1;
            self.s(c, f::TURN_PROG_POS, p);
            return;
        }
        // turn_to_script
        self.s(c, f::AR_ANIM_INDEX, 0);
        self.s(c, f::LOGIC, L_SCRIPT as u16);
        self.logic_script();
    }

    fn cursor_logic(&mut self) {
        let (x, y) = (self.mouse.x, self.mouse.y);
        self.logic_cursor(self.compact, x, y);
    }

    fn talk(&mut self) {
        let c = self.compact;
        // the speech finishes when the timer runs out, not when the
        // animation does; the player may click to skip
        if self.mouse_was_clicked() {
            let cur = self.vars[CUR_ID as usize] as u16 as u32;
            if CLICK_TABLE.contains(&cur) {
                if self.sys.system_flags & SF_ALLOW_SPEECH != 0 && !self.audio_speech_finished() {
                    self.audio_stop_speech();
                }
                let text_id = self.g(c, f::SP_TEXT_ID);
                if text_id > 0 && text_id < 0xFFFF {
                    let t = self.cpt_start(text_id as u32);
                    self.s(t, f::STATUS, 0);
                }
                if self.grafix_ptr(c).is_some() {
                    let stand = self.g(c, f::GET_TO_FLAG);
                    self.s(c, f::FRAME, stand); // set character to stand
                    self.s(c, f::GRAFIX_PROG_ID, 0);
                }
                self.s(c, f::LOGIC, L_SCRIPT as u16);
                self.logic_script();
                return;
            }
        }

        // with speech, wait for it to finish before finishing animations
        if self.g(c, f::SP_TEXT_ID) == 0xFFFF && self.audio_speech_finished() {
            self.s(c, f::LOGIC, L_SCRIPT as u16);
            if self.grafix_ptr(c).is_some() {
                let stand = self.g(c, f::GET_TO_FLAG);
                self.s(c, f::FRAME, stand);
                self.s(c, f::GRAFIX_PROG_ID, 0);
            }
            self.logic_script();
            return;
        }

        if let Some(gp) = self.grafix_ptr(c) {
            if self.w(gp) != 0 && (self.g(c, f::SP_TIME) != 3 || !self.audio_speech_finished()) {
                // the animation is forced to finish 3 game cycles before the
                // speech does, because it looks good
                let frame = self.w(gp + 2).wrapping_add(self.g(c, f::OFFSET));
                self.s(c, f::FRAME, frame);
                let p = self.g(c, f::GRAFIX_PROG_POS) + 3;
                self.s(c, f::GRAFIX_PROG_POS, p);
            } else {
                // ran out of frames or speech finished: stand still
                let stand = self.g(c, f::GET_TO_FLAG);
                self.s(c, f::FRAME, stand);
                self.s(c, f::GRAFIX_PROG_ID, 0);
            }
        }

        if self.audio_speech_finished() {
            let t = self.g(c, f::SP_TIME).wrapping_sub(1);
            self.s(c, f::SP_TIME, t);
        }

        if self.g(c, f::SP_TIME) == 0 {
            // speech has finished
            let text_id = self.g(c, f::SP_TEXT_ID);
            if text_id != 0 {
                if let Some(t) = self.fetch(text_id as u32) {
                    self.s(t, f::STATUS, 0); // kill the text
                }
            }
            self.s(c, f::LOGIC, L_SCRIPT as u16);
            self.logic_script();
        }
    }

    fn listen(&mut self) {
        // stay in this mode until the id in flag leaves L_TALK
        let c = self.compact;
        let other = self.cpt_start(self.g(c, f::FLAG) as u32);
        if self.g(other, f::LOGIC) as u32 == L_TALK {
            return;
        }
        self.s(c, f::LOGIC, L_SCRIPT as u16);
        self.logic_script();
    }

    fn stopped(&mut self) {
        // waiting for another mega to move, or give up trying
        let c = self.compact;
        if let Some(other) = self.fetch(self.g(c, f::WAITING_FOR) as u32) {
            if self.g(other, f::MOOD) == 0 && self.is_collision(other) {
                return;
            }
        }
        // we are free: restart the script one level below
        let m = self.g(c, f::MODE);
        self.set_sub(c, m.wrapping_sub(2), 0);
        self.s(c, f::WAITING_FOR, 0xFFFF);
        self.s(c, f::LOGIC, L_SCRIPT as u16);
        self.logic_script();
    }

    fn choose(&mut self) {
        // remain in this mode until the player selects some text
        if self.vars[THE_CHOSEN_ONE as usize] == 0 {
            return;
        }
        self.fn_no_human(0, 0, 0); // kill mouse again
        self.sys.system_flags &= !SF_CHOOSING; // restore save/restore
        let c = self.compact;
        self.s(c, f::LOGIC, L_SCRIPT as u16);
        self.logic_script();
    }

    fn frames(&mut self) {
        let c = self.compact;
        if self.g(c, f::SYNC) == 0 {
            self.simple_anim();
        } else {
            self.s(c, f::DOWN_FLAG, 0); // return 'ok' to script
            self.s(c, f::LOGIC, L_SCRIPT as u16);
            self.logic_script();
        }
    }

    fn pause_logic(&mut self) {
        let c = self.compact;
        let fl = self.g(c, f::FLAG).wrapping_sub(1);
        self.s(c, f::FLAG, fl);
        if fl != 0 {
            return;
        }
        self.s(c, f::LOGIC, L_SCRIPT as u16);
        self.logic_script();
    }

    fn wait_sync(&mut self) {
        let c = self.compact;
        if self.g(c, f::SYNC) == 0 {
            return;
        }
        self.s(c, f::LOGIC, L_SCRIPT as u16);
        self.logic_script();
    }

    /// Follow an animation sequence module, ignoring its coordinates.
    pub(crate) fn simple_anim(&mut self) {
        let c = self.compact;
        let mut gp = self.grafix_ptr(c).expect("simple anim program");
        while self.w(gp) != 0 {
            let p = self.g(c, f::GRAFIX_PROG_POS) + 3;
            self.s(c, f::GRAFIX_PROG_POS, p);
            if self.w(gp) as u32 != SEND_SYNC {
                let frame = self.w(gp + 2);
                if frame >= 64 {
                    self.s(c, f::FRAME, frame);
                } else {
                    self.s(c, f::FRAME, frame.wrapping_add(self.g(c, f::OFFSET)));
                }
                return;
            }
            let target = self.cpt_start(self.w(gp + 1) as u32);
            let sync = self.w(gp + 2);
            gp += 3;
            self.s(target, f::SYNC, sync);
        }
        self.s(c, f::DOWN_FLAG, 0);
        self.s(c, f::LOGIC, L_SCRIPT as u16);
        self.logic_script();
    }

    /// Does the current compact collide with `other`?
    pub(crate) fn is_collision(&self, other: usize) -> bool {
        let c = self.compact;
        let this_ms = self.mega_set(c);
        let other_ms = self.mega_set(other);
        let dir = self.g(c, f::DIR) as u32;
        let xc = self.g(c, f::XCOOD);
        let yc = self.g(c, f::YCOOD);

        // target's base coordinates (16-bit arithmetic, as in the original)
        let mut ox = self.g(other, f::XCOOD) & !7;
        let oy = self.g(other, f::YCOOD) & !7;
        if dir == UPY || dir == DOWNY {
            ox = ox.wrapping_sub(self.w(this_ms + f::MS_COL_OFFSET));
            ox = ox.wrapping_add(self.w(other_ms + f::MS_COL_OFFSET));
        }
        // the C compares in int after promotion, so do the sums wide
        let ox = ox as i32;
        let oy = oy as i32;
        let (xc, yc) = (xc as i32, yc as i32);
        if dir == UPY || dir == DOWNY {
            if ox + (self.w(other_ms + f::MS_COL_WIDTH) as i32) < xc {
                return false; // other is left of us
            }
            if ox - (self.w(this_ms + f::MS_COL_WIDTH) as i32) >= xc {
                return false; // other is right of us
            }
            if dir == UPY {
                oy + 8 == yc || oy + 16 == yc
            } else {
                oy - 8 == yc || oy - 16 == yc
            }
        } else {
            if oy != yc {
                return false;
            }
            if dir == LEFTY {
                let lc = self.w(other_ms + f::MS_LAST_CHR) as i32;
                ox + lc == xc || ox + lc - 8 == xc
            } else {
                let lc = self.w(this_ms + f::MS_LAST_CHR) as i32;
                ox - lc == xc || ox - lc - 8 == xc
            }
        }
    }

    pub(crate) fn run_get_off(&mut self) {
        let get_off = self.vars[GET_OFF as usize];
        self.vars[GET_OFF as usize] = 0;
        if get_off != 0 {
            self.script((get_off & 0xFFFF) as u16, (get_off >> 16) as u16);
        }
    }

    pub(crate) fn stop_and_wait(&mut self) {
        let c = self.compact;
        let m = self.g(c, f::MODE) + 4;
        self.s(c, f::MODE, m);
        let stop = self.g(c, f::STOP_SCRIPT);
        self.set_sub(c, m, stop);
        self.set_sub(c, m + 2, 0);
        self.s(c, f::LOGIC, L_SCRIPT as u16);
        self.logic_script();
    }

    fn push(&mut self, v: u32) {
        assert!(self.stack.len() < 19, "script stack overflow");
        self.stack.push(v);
    }

    fn pop(&mut self) -> u32 {
        self.stack.pop().expect("no items on script stack to pop")
    }

    /// Runs `scr_num` with `cpt` as the current compact (mouse handlers).
    pub(crate) fn mouse_script(&mut self, scr_num: u32, cpt: usize) -> u16 {
        let saved = self.compact;
        self.compact = cpt;
        let ret = self.script((scr_num & 0xFFFF) as u16, (scr_num >> 16) as u16);
        self.compact = saved;
        if scr_num == MENU_SELECT || (LINC_MENU_SELECT..=DOC_MENU_SELECT).contains(&scr_num) {
            // HACK from ScummVM (bug #8627): after clicking an inventory
            // item, update the pointer now instead of when it moves away
            self.fn_cross_mouse(0, 0, 0);
        }
        ret
    }

    /// The script interpreter. `script_no` bits 0-11 are the script number,
    /// 12-15 the module. Returns 0 when the script finished, otherwise the
    /// offset to continue from.
    pub fn script(&mut self, script_no: u16, mut offset: u16) -> u16 {
        'restart: loop {
            let module_no = (script_no >> 12) as usize;
            if self.modules[module_no].is_none() {
                let file = module_no as u32 + F_MODULE_0;
                let bytes = self.disk.load(file as u16).expect("script module loads");
                let words = bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
                self.modules[module_no] = Some(std::rc::Rc::new(words));
            }

            // WORKAROUND for bug #5567: using the dark glasses on Trevor
            // repeatedly wrecks his mode; pretend the item is one the script
            // handles
            if script_no as u32 == TREVOR_SPEECH && self.vars[OBJECT_HELD as usize] == IDO_SHADES {
                self.vars[OBJECT_HELD as usize] = IDO_GLASS;
            }

            let module = self.modules[module_no].clone().unwrap();
            let mut pc: usize = if offset != 0 {
                offset as usize
            } else {
                module[(script_no & 0x0FFF) as usize] as usize
            };
            let mut a: u32 = 0;
            let mut b: u32 = 0;
            let mut c: u32 = 0;
            loop {
                let command = module[pc];
                log::trace!(target: "script", "{:#06x} {script_no:#06x} @{pc}: {command} {} stack {:?}", self.vars[CUR_ID as usize], module.get(pc + 1).copied().unwrap_or(0), &self.stack);
                pc += 1;
                match command {
                    0 => {
                        // push_variable
                        let v = self.vars[module[pc] as usize / 4];
                        pc += 1;
                        self.push(v);
                    }
                    1 => {
                        // less_than
                        a = self.pop();
                        b = self.pop();
                        self.push((a > b) as u32);
                    }
                    2 => {
                        // push_number
                        let v = module[pc] as u32;
                        pc += 1;
                        self.push(v);
                    }
                    3 => {
                        // not_equal
                        a = self.pop();
                        b = self.pop();
                        self.push((a != b) as u32);
                    }
                    4 => {
                        // if_and
                        a = self.pop();
                        b = self.pop();
                        self.push((a != 0 && b != 0) as u32);
                    }
                    5 => {
                        // skip_zero
                        let s = module[pc];
                        pc += 1;
                        a = self.pop();
                        if a == 0 {
                            pc += s as usize / 2;
                        }
                    }
                    6 => {
                        // pop_var (the original leaves the value in b too,
                        // which a following call_mcode can pass on)
                        b = self.pop();
                        self.vars[module[pc] as usize / 4] = b;
                        pc += 1;
                    }
                    7 => {
                        // minus
                        a = self.pop();
                        b = self.pop();
                        self.push(b.wrapping_sub(a));
                    }
                    8 => {
                        // plus
                        a = self.pop();
                        b = self.pop();
                        self.push(b.wrapping_add(a));
                    }
                    9 => {
                        // skip_always
                        let s = module[pc];
                        pc += 1;
                        pc += s as usize / 2;
                    }
                    10 => {
                        // if_or
                        a = self.pop();
                        b = self.pop();
                        self.push((a != 0 || b != 0) as u32);
                    }
                    11 => {
                        // call_mcode
                        let n = module[pc];
                        pc += 1;
                        assert!(n <= 3);
                        // as the original: a holds the count until popped,
                        // and b and c keep whatever the last command left
                        // in them when fewer are pushed (Foster's arrival on
                        // the ground floor sends Joey's sync with one
                        // argument and relies on the stale b)
                        a = n as u32;
                        // the original falls through from 3 to 1
                        if n >= 3 {
                            c = self.pop();
                        }
                        if n >= 2 {
                            b = self.pop();
                        }
                        if n >= 1 {
                            a = self.pop();
                        }
                        let mcode = module[pc] / 4;
                        pc += 1;
                        let saved = self.compact;
                        // ScummVM's Debug::mcode
                        log::trace!(target: "mcode", "{:#06x} {script_no:#06x}: mcode {mcode}({a}, {b}, {c}) {n} args", self.vars[CUR_ID as usize]);
                        let ret = self.mcode(mcode, a, b, c);
                        self.compact = saved;
                        if !ret || self.quit {
                            return pc as u16;
                        }
                    }
                    12 => {
                        // more_than
                        a = self.pop();
                        b = self.pop();
                        self.push((a < b) as u32);
                    }
                    14 => {
                        // switch (the original leaves the number of cases in c)
                        let mut s = module[pc];
                        c = s as u32;
                        pc += 1;
                        a = self.pop();
                        loop {
                            if a == module[pc] as u32 {
                                pc += module[pc + 1] as usize / 2;
                                pc += 1;
                                break;
                            }
                            pc += 2;
                            s -= 1;
                            if s == 0 {
                                break;
                            }
                        }
                        if s == 0 {
                            pc += module[pc] as usize / 2; // use the default
                        }
                    }
                    15 => {
                        // push_offset
                        let at = self.elem(self.compact, module[pc]);
                        pc += 1;
                        let v = self.w(at) as u32;
                        self.push(v);
                    }
                    16 => {
                        // pop_offset: pop a value into a compact
                        let at = self.elem(self.compact, module[pc]);
                        pc += 1;
                        let v = self.pop() as u16;
                        self.sw(at, v);
                    }
                    17 => {
                        // is_equal
                        a = self.pop();
                        b = self.pop();
                        self.push((a == b) as u32);
                    }
                    18 => {
                        // skip_nz
                        let t = module[pc] as i16;
                        pc += 1;
                        a = self.pop();
                        if a != 0 {
                            pc = (pc as isize + (t / 2) as isize) as usize;
                        }
                    }
                    13 | 19 => {
                        // script_exit
                        return 0;
                    }
                    20 => {
                        // restart_script
                        offset = 0;
                        continue 'restart;
                    }
                    other => panic!("unknown script command {other}"),
                }
            }
        }
    }
}
