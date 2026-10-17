//! The intro. Port of ScummVM's engines/sky/intro.cpp (and intro.h),
//! GPL-3.0-or-later like this crate.
//!
//! The intro is a list of commands: show a still, fade the palette, play
//! a "flirt" sequence (screen.rs `start_sequence`), play a voice or a
//! background sound, start music, wait. The CD version narrates it with
//! voices; the floppy version (ScummVM's `alt_intro`, whose data the CD
//! carries too) scrolls down to the city and shows a few lines of text.
//!
//! ScummVM's `Intro` class keeps its own buffers; here they are the
//! `Intro` struct below, passed to the `impl Sky` functions that stand in
//! for its methods.
//!
//! Two differences, both for the terminal:
//! - Text the floppy intro draws straight into the screen is also handed
//!   to the front end as a `TextOverlay`, so it is set as real type, and
//!   its pixels are drawn only when `screen.text_pixels` is on, as for
//!   text sprites (screen.rs `draw_sprite_data`).
//! - Without an audio device ScummVM's mixer would report each voice as
//!   finished at once and the CD intro would race through. Here a voice
//!   that cannot be heard lasts as long as it would have played, so the
//!   pictures keep their pace.

use std::sync::Arc;

use super::defs::*;
use super::disk::HEADER_SIZE;
use super::host::{Action, TextOverlay};
use super::sky::Sky;
use super::text::Align;

const SHOWSCREEN: u16 = 0;
const COMMANDEND: u16 = 0; // end of COMMANDFLIRT block
const FADEUP: u16 = 1; // fade up palette
const FADEDOWN: u16 = 2;
const DELAY: u16 = 3;
const DOFLIRT: u16 = 4; // start flirt sequence (and wait for it to finish)
const SCROLLFLIRT: u16 = 5; // start special floppy intro flirt sequence (and wait for it)
const COMMANDFLIRT: u16 = 6; // start flirt sequence and wait for it, while processing command block
const BGFLIRT: u16 = 7; // start flirt sequence without waiting for it
const WAITFLIRT: u16 = 8; // wait for sequence started by BGFLIRT
const STOPFLIRT: u16 = 9;
const STARTMUSIC: u16 = 10;
const WAITMUSIC: u16 = 11;
const PLAYVOICE: u16 = 12;
const WAITVOICE: u16 = 13;
const LOADBG: u16 = 14; // load new background sound
const PLAYBG: u16 = 15; // play background sound
const LOOPBG: u16 = 16; // loop background sound
const STOPBG: u16 = 17; // stop background sound
const CLEARBOTTOM: u16 = 18; // clear the screen
const SEQEND: u16 = 65535; // end of intro sequence

// Modifier flag for SHOWSCREEN when we want the image to cover the entire
// screen.
const FULLSCREEN: u16 = 0x8000;

const IC_PREPARE_TEXT: u16 = 20; // commands used in COMMANDFLIRT block
const IC_SHOW_TEXT: u16 = 21;
const IC_REMOVE_TEXT: u16 = 22;
const IC_MAKE_SOUND: u16 = 23;
const IC_FX_VOLUME: u16 = 24;

const W: usize = GAME_SCREEN_WIDTH as usize;
const FRAME_SIZE: usize = (GAME_SCREEN_WIDTH * GAME_SCREEN_HEIGHT) as usize;
const INTRO_TEXT_WIDTH: u16 = 128;

/// Text prepared by IC_PREPARE_TEXT (ScummVM's `_textBuf`).
struct TextBuf {
    /// the text sprite: a header, then width * height pixels
    data: Vec<u8>,
    lines: Vec<String>,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}

/// What IC_SHOW_TEXT drew over (ScummVM's `_saveBuf`), and the overlay it
/// handed the front end.
struct SaveBuf {
    pixels: Vec<u8>,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    overlay: TextOverlay,
}

/// The state of ScummVM's `Intro` object.
#[derive(Default)]
struct Intro {
    text_buf: Option<TextBuf>,
    save_buf: Option<SaveBuf>,
    /// the background sound loaded by LOADBG (`_bgBuf`)
    bg_buf: Option<Arc<[u8]>>,
    /// `_relDelay`: the time escDelay waits until, in host milliseconds
    rel_delay: i64,
    /// without an audio device: when the last voice would have ended
    silent_voice_end: u64,
}

/// A word of a sequence table, read where `pos` points, which then moves on.
fn next(data: &[u16], pos: &mut usize) -> u16 {
    let v = data[*pos];
    *pos += 1;
    v
}

impl Sky {
    /// `Intro::doIntro` for the CD intro (the floppy one when the data is
    /// not the CD version's). Returns true when the intro played to the
    /// end, false when skipped.
    pub(crate) fn do_intro(&mut self) -> bool {
        self.do_intro_alt(false)
    }

    /// `Intro::doIntro(floppyIntro)`, with the clean-up of `~Intro`.
    pub fn do_intro_alt(&mut self, floppy_intro: bool) -> bool {
        let mut it = Intro::default();
        let played = self.intro_run(&mut it, floppy_intro);

        // ~Intro
        if self.sequence_running() {
            self.stop_sequence();
        }
        self.audio.sound.stop_intro_bg();
        // our addition: no intro text may stay with the front end
        if let Some(save) = it.save_buf.take() {
            self.screen.texts_shown.retain(|t| *t != save.overlay);
        }
        played
    }

    fn intro_run(&mut self, it: &mut Intro, mut floppy_intro: bool) -> bool {
        if !self.disk.is_cd() {
            floppy_intro = true;
        }

        self.audio_load_section(0);

        if !self.esc_delay(it, 3000) {
            return false;
        }

        if floppy_intro {
            self.audio_start_music(1);
        }

        let mut pos = 0;
        while MAIN_INTRO_SEQ[pos] != SEQEND {
            if !self.next_part(it, MAIN_INTRO_SEQ, &mut pos) {
                return false;
            }
        }
        let seq = if floppy_intro { FLOPPY_INTRO_SEQ } else { CD_INTRO_SEQ };

        let mut pos = 0;
        while seq[pos] != SEQEND {
            if !self.next_part(it, seq, &mut pos) {
                return false;
            }
        }
        true
    }

    fn next_part(&mut self, it: &mut Intro, data: &[u16], pos: &mut usize) -> bool {
        // return false means cancel intro
        let command = next(data, pos);
        match command & 0x7fff {
            SHOWSCREEN => {
                let file = next(data, pos);
                self.show_screen_file(file, command & FULLSCREEN != 0);
                true
            }
            FADEUP => {
                let file = next(data, pos);
                self.palette_fade_up_file(file);
                it.rel_delay += 32 * 20; // hack: the screen uses a separate delay function for the
                                         // blocking fadeups. So add 32*20 msecs to out delay counter.
                true
            }
            FADEDOWN => {
                self.fn_fade_down(0);
                it.rel_delay += 32 * 20; // hack: see above.
                true
            }
            DELAY => {
                let ms = next(data, pos);
                self.esc_delay(it, ms as u32)
            }
            DOFLIRT => {
                let file = next(data, pos);
                self.start_sequence(file);
                while self.sequence_running() {
                    if !self.esc_delay(it, 50) {
                        return false;
                    }
                }
                true
            }
            SCROLLFLIRT => self.floppy_scroll_flirt(it),
            COMMANDFLIRT => self.command_flirt(it, data, pos),
            STOPFLIRT => {
                self.stop_sequence();
                true
            }
            STARTMUSIC => {
                let n = next(data, pos);
                self.audio_start_music(n);
                true
            }
            WAITMUSIC => {
                while self.audio.music.music_is_playing() {
                    if !self.esc_delay(it, 50) {
                        return false;
                    }
                }
                true
            }
            BGFLIRT => {
                let file = next(data, pos);
                self.start_sequence(file);
                true
            }
            WAITFLIRT => {
                while self.sequence_running() {
                    if !self.esc_delay(it, 50) {
                        return false;
                    }
                }
                true
            }
            PLAYVOICE => {
                if !self.esc_delay(it, 200) {
                    return false;
                }
                let file = next(data, pos);
                let Some(v_data) = self.disk.load(file) else {
                    log::warn!("intro: can't load voice {file}");
                    return true;
                };
                if !self.audio.audio.available() {
                    // 8-bit mono at 11025 Hz, header included, as it would play
                    it.silent_voice_end = self.host.millis() + v_data.len() as u64 * 1000 / 11025;
                }
                self.audio.sound.play_intro_voice(v_data);
                true
            }
            WAITVOICE => {
                while self.intro_voice_active(it) {
                    if !self.esc_delay(it, 50) {
                        return false;
                    }
                }
                true
            }
            LOADBG => {
                self.audio.sound.stop_intro_bg();
                let file = next(data, pos);
                it.bg_buf = self.disk.load(file).map(Arc::from);
                if it.bg_buf.is_none() {
                    log::warn!("intro: can't load background sound {file}");
                }
                true
            }
            LOOPBG | PLAYBG => {
                // play_intro_bg stops SOUND_BG first, as ScummVM does
                if let Some(bg) = &it.bg_buf {
                    self.audio.sound.play_intro_bg(bg.clone(), command == LOOPBG);
                }
                true
            }
            STOPBG => {
                self.audio.sound.stop_intro_bg();
                true
            }
            CLEARBOTTOM => {
                let s = &mut self.screen;
                let bottom = (GAME_SCREEN_HEIGHT * GAME_SCREEN_WIDTH) as usize;
                let end = (FULL_SCREEN_HEIGHT * GAME_SCREEN_WIDTH) as usize;
                s.current[bottom..end].fill(0);
                s.display[bottom..end].copy_from_slice(&s.current[bottom..end]);
                self.update_screen();
                true
            }
            _ => panic!("Unknown intro command {command:X}"),
        }
    }

    /// `_mixer->isSoundHandleActive(_voice)`, or, with no device to play
    /// on, whether the voice would still be playing.
    fn intro_voice_active(&self, it: &Intro) -> bool {
        if self.audio.audio.available() {
            self.audio.sound.intro_voice_active()
        } else {
            self.host.millis() < it.silent_voice_end
        }
    }

    fn floppy_scroll_flirt(&mut self, it: &mut Intro) -> bool {
        let mut scroll_screen = vec![0u8; FRAME_SIZE * 2];
        scroll_screen[FRAME_SIZE..].copy_from_slice(&self.screen.current[..FRAME_SIZE]);
        let mut scroll_pos = FRAME_SIZE;
        let vga_data = self.disk.load(60100).unwrap_or_default();
        let diff_data = self.disk.load(60101).unwrap_or_default();
        let byte = |d: &[u8], i: usize| *d.get(i).unwrap_or(&0);
        let frame_num = u16::from_le_bytes([byte(&diff_data, 0), byte(&diff_data, 1)]);
        let mut diff_ptr = 2usize;
        let mut vga_ptr = 0usize;
        let mut do_continue = true;

        let mut frame_cnt = 1u16;
        while frame_cnt < frame_num && do_continue {
            let scroll_val = byte(&diff_data, diff_ptr);
            diff_ptr += 1;
            if scroll_val != 0 {
                scroll_pos = scroll_pos.saturating_sub(scroll_val as usize * W);
            }

            let mut scr_pos = 0usize;
            while scr_pos < FRAME_SIZE {
                loop {
                    let nr_to_skip = byte(&diff_data, diff_ptr);
                    diff_ptr += 1;
                    scr_pos += nr_to_skip as usize;
                    if nr_to_skip != 255 {
                        break;
                    }
                }
                loop {
                    let nr_to_do = byte(&diff_data, diff_ptr) as usize;
                    diff_ptr += 1;
                    for i in 0..nr_to_do {
                        if let Some(p) = scroll_screen.get_mut(scroll_pos + scr_pos + i) {
                            *p = byte(&vga_data, vga_ptr + i);
                        }
                    }
                    scr_pos += nr_to_do;
                    vga_ptr += nr_to_do;
                    if nr_to_do != 255 {
                        break;
                    }
                }
            }
            self.screen.display[..FRAME_SIZE].copy_from_slice(&scroll_screen[scroll_pos..scroll_pos + FRAME_SIZE]);
            self.update_screen();
            if !self.esc_delay(it, 60) {
                do_continue = false;
            }
            frame_cnt += 1;
        }
        self.screen.current[..FRAME_SIZE].copy_from_slice(&scroll_screen[scroll_pos..scroll_pos + FRAME_SIZE]);
        do_continue
    }

    fn command_flirt(&mut self, it: &mut Intro, data: &[u16], pos: &mut usize) -> bool {
        let file = next(data, pos);
        self.start_sequence(file);

        while data[*pos] != COMMANDEND || self.sequence_running() {
            while self.screen.seq.frames_left < data[*pos] as u32 {
                *pos += 1;
                let command = next(data, pos);
                match command {
                    IC_PREPARE_TEXT => {
                        let nr = next(data, pos);
                        let text = self.get_text(nr as u32);
                        let t = self.display_text(text, Align::Center, INTRO_TEXT_WIDTH, 255);
                        let rd = |i: usize| u16::from_le_bytes([t.data[i * 2], t.data[i * 2 + 1]]);
                        it.text_buf = Some(TextBuf {
                            width: rd(3),
                            height: rd(4),
                            data: t.data,
                            lines: t.lines,
                            x: 0,
                            y: 0,
                        });
                    }
                    IC_SHOW_TEXT => {
                        let x = next(data, pos);
                        let y = next(data, pos);
                        if let Some(t) = it.text_buf.as_mut() {
                            t.x = x;
                            t.y = y;
                        }
                        self.show_text_buf(it);
                    }
                    IC_REMOVE_TEXT => self.restore_screen(it),
                    IC_MAKE_SOUND => {
                        let (sound, volume) = (data[*pos], data[*pos + 1]);
                        self.audio.sound.play_sound(sound, volume, 0);
                        *pos += 2;
                    }
                    IC_FX_VOLUME => {
                        let volume = next(data, pos);
                        self.audio.sound.play_sound(1, volume, 0);
                    }
                    _ => panic!("Unknown FLIRT command {command:X}"),
                }
            }

            if !self.esc_delay(it, 50) {
                self.stop_sequence();
                return false;
            }
        }

        *pos += 1; // move pointer over "COMMANDEND"
        true
    }

    fn show_text_buf(&mut self, it: &mut Intro) {
        let Some(t) = it.text_buf.as_ref() else { return };
        let (x, y, width, height) = (t.x as usize, t.y as usize, t.width as usize, t.height as usize);
        let s = &mut self.screen;
        let mut save = Vec::with_capacity(width * height);
        for cnty in 0..height {
            let row = (y + cnty) * W + x;
            for cntx in 0..width {
                let at = row + cntx;
                let Some(&old) = s.current.get(at) else {
                    save.push(0);
                    continue;
                };
                save.push(old);
                let p = *t.data.get(HEADER_SIZE + cnty * width + cntx).unwrap_or(&0);
                if p != 0 && s.text_pixels {
                    s.current[at] = p;
                }
            }
        }
        for cnty in 0..height {
            let row = (y + cnty) * W + x;
            let end = (row + width).min(s.current.len());
            if row < end {
                s.display[row..end].copy_from_slice(&s.current[row..end]);
            }
        }
        // our addition: the words, for the terminal to set as type
        let overlay = TextOverlay {
            x: x as i32,
            y: y as i32,
            width: t.width,
            line_height: t.height / t.lines.len().max(1) as u16,
            lines: t.lines.clone(),
            color: 255,
            color_rgb: [0, 0, 0],
            centered: true,
        };
        if let Some(old) = it.save_buf.take() {
            s.texts_shown.retain(|o| *o != old.overlay);
        }
        s.texts_shown.push(overlay.clone());
        it.save_buf = Some(SaveBuf { pixels: save, x: t.x, y: t.y, width: t.width, height: t.height, overlay });
    }

    fn restore_screen(&mut self, it: &mut Intro) {
        let Some(sb) = it.save_buf.take() else { return };
        let (x, y, width) = (sb.x as usize, sb.y as usize, sb.width as usize);
        let s = &mut self.screen;
        for cnt in 0..sb.height as usize {
            let row = (y + cnt) * W + x;
            let end = (row + width).min(s.current.len());
            if row < end {
                let src = &sb.pixels[cnt * width..cnt * width + (end - row)];
                s.current[row..end].copy_from_slice(src);
                s.display[row..end].copy_from_slice(src);
            }
        }
        s.texts_shown.retain(|o| *o != sb.overlay);
    }

    /// `Intro::escDelay`: waits until `msecs` after the last wait ended,
    /// running the sequence and showing the screen meanwhile. False when
    /// the player skips (Escape) or quits.
    fn esc_delay(&mut self, it: &mut Intro, msecs: u32) -> bool {
        if it.rel_delay == 0 {
            // first call, init with system time
            it.rel_delay = self.host.millis() as i64;
        }

        it.rel_delay += msecs as i64; // now wait until millis() >= rel_delay

        loop {
            self.pump_events();
            if self.action == Action::Skip || self.quit {
                self.action = Action::None;
                return false;
            }
            // other keys are dropped, as ScummVM's event loop here does
            self.action = Action::None;
            self.key_pressed = false;

            let n_delay = (it.rel_delay - self.host.millis() as i64).clamp(0, 20);

            self.host.sleep(n_delay as u32);

            self.process_sequence();
            self.update_screen();
            if n_delay != 20 {
                return true;
            }
        }
    }
}

//CD intro file defines
#[allow(dead_code)]
mod files {
    pub const CDV_00: u16 = 59500;
    pub const CD_PAL: u16 = 59501;
    pub const CD_1_LOG: u16 = 59502;
    pub const CD_1: u16 = 59503;
    pub const CDV_01: u16 = 59504;
    pub const CDV_02: u16 = 59505;
    pub const CD_2: u16 = 59506;
    pub const CDV_03: u16 = 59507;
    pub const CDV_04: u16 = 59508;
    pub const CD_3: u16 = 59509;
    pub const CDV_05: u16 = 59510;
    pub const CDV_06: u16 = 59511;
    pub const CD_5: u16 = 59512;
    pub const CDV_07: u16 = 59513;
    pub const CDV_08: u16 = 59514;
    pub const CDV_09: u16 = 59515;
    pub const CD_7: u16 = 59516;
    pub const CDV_10: u16 = 59518;
    pub const CD_11: u16 = 59519;
    pub const CDV_11: u16 = 59520;
    pub const CD_11_PAL: u16 = 59521;
    pub const CD_11_LOG: u16 = 59522;
    pub const CDV_12: u16 = 59523;
    pub const CD_13: u16 = 59524;
    pub const CDV_13: u16 = 59525;
    pub const CDV_14: u16 = 59527;
    pub const CDV_15: u16 = 59528;
    pub const CD_15_PAL: u16 = 59529;
    pub const CD_15_LOG: u16 = 59530;
    pub const CDV_16: u16 = 59531;
    pub const CD_17_LOG: u16 = 59532;
    pub const CD_17: u16 = 59533;
    pub const CDV_17: u16 = 59534;
    pub const CDV_18: u16 = 59535;
    pub const CDV_19: u16 = 59536;
    pub const CD_19_PAL: u16 = 59537;
    pub const CD_19_LOG: u16 = 59538;
    pub const CDV_20: u16 = 59539;
    pub const CD_20_LOG: u16 = 59540;
    pub const CDV_21: u16 = 59541;
    pub const CD_21_LOG: u16 = 59542;
    pub const CDV_22: u16 = 59545;
    pub const CDV_23: u16 = 59546;
    pub const CD_23_PAL: u16 = 59547;
    pub const CD_24_LOG: u16 = 59550;
    pub const CDV_24: u16 = 59551;
    pub const CDV_25: u16 = 59554;
    pub const CDV_26: u16 = 59556;
    pub const CD_27: u16 = 59557;
    pub const CDV_27: u16 = 59558;
    pub const CD_27_PAL: u16 = 59559;
    pub const CD_27_LOG: u16 = 59560;
    pub const CDV_28: u16 = 59561;
    pub const CDV_29: u16 = 59562;
    pub const CDV_30: u16 = 59563;
    pub const CDV_31: u16 = 59565;
    pub const CDV_32: u16 = 59566;
    pub const CDV_33: u16 = 59567;
    pub const CDV_34: u16 = 59568;
    pub const CD_35: u16 = 59569;
    pub const CDV_35: u16 = 59570;
    pub const CD_35_PAL: u16 = 59571;
    pub const CD_35_LOG: u16 = 59572;
    pub const CDV_36: u16 = 59574;
    pub const CD_37: u16 = 59575;
    pub const CDV_37: u16 = 59576;
    pub const CD_37_PAL: u16 = 59577;
    pub const CD_37_LOG: u16 = 59578;
    pub const CDV_38: u16 = 59579;
    pub const CDV_39: u16 = 59581;
    pub const CDV_40: u16 = 59583;
    pub const CD_40_PAL: u16 = 59584;
    pub const CD_40_LOG: u16 = 59585;
    pub const CDV_41: u16 = 59587;
    pub const CDV_42: u16 = 59588;
    pub const CD_43: u16 = 59589;
    pub const CDV_43: u16 = 59590;
    pub const CD_43_PAL: u16 = 59591;
    pub const CD_43_LOG: u16 = 59592;
    pub const CDV_44: u16 = 59594;
    pub const CD_45: u16 = 59595;
    pub const CDV_45: u16 = 59596;
    pub const CD_45_PAL: u16 = 59597;
    pub const CD_45_LOG: u16 = 59598;
    pub const CDV_46: u16 = 59600;
    pub const CDV_47: u16 = 59602;
    pub const CD_47_PAL: u16 = 59603;
    pub const CD_47_LOG: u16 = 59604;
    pub const CD_48: u16 = 59605;
    pub const CDV_48: u16 = 59606;
    pub const CD_48_PAL: u16 = 59607;
    pub const CD_48_LOG: u16 = 59608;
    pub const CD_49: u16 = 59609;
    pub const CDV_49: u16 = 59610;
    pub const CD_50: u16 = 59611;
    pub const CDV_50: u16 = 59612;
    pub const CDV_51: u16 = 59613;
    pub const CDV_52: u16 = 59614;
    pub const CDV_53: u16 = 59615;
    pub const CDV_54: u16 = 59616;
    pub const CDV_55: u16 = 59618;
    pub const CD_55_PAL: u16 = 59619;
    pub const CD_55_LOG: u16 = 59620;
    pub const CDV_56: u16 = 59621;
    pub const CDV_57: u16 = 59622;
    pub const CD_58: u16 = 59623;
    pub const CDV_58: u16 = 59624;
    pub const CD_58_PAL: u16 = 59625;
    pub const CD_58_LOG: u16 = 59626;
    pub const CDV_59: u16 = 59627;
    pub const CDV_60: u16 = 59628;
    pub const CDV_61: u16 = 59629;
    pub const CDV_62: u16 = 59630;
    pub const CDV_63: u16 = 59631;
    pub const CDV_64: u16 = 59632;
    pub const CDV_65: u16 = 59633;
    pub const CDV_66: u16 = 59635;
    pub const CD_66_PAL: u16 = 59636;
    pub const CD_66_LOG: u16 = 59637;
    pub const CDV_67: u16 = 59639;
    pub const CD_67_PAL: u16 = 59640;
    pub const CD_67_LOG: u16 = 59641;
    pub const CDV_68: u16 = 59642;
    pub const CD_69: u16 = 59643;
    pub const CDV_69: u16 = 59644;
    pub const CD_69_PAL: u16 = 59645;
    pub const CD_69_LOG: u16 = 59646;
    pub const CDV_70: u16 = 59647;
    pub const CDV_71: u16 = 59648;
    pub const CDV_72: u16 = 59649;
    pub const CD_72_PAL: u16 = 59650;
    pub const CD_72_LOG: u16 = 59651;
    pub const CD_73_PAL: u16 = 59652;
    pub const CD_73_LOG: u16 = 59653;
    pub const CDV_73: u16 = 59654;
    pub const CDV_74: u16 = 59655;
    pub const CDV_75: u16 = 59656;
    pub const CD_76_PAL: u16 = 59657;
    pub const CD_76_LOG: u16 = 59658;
    pub const CDV_76: u16 = 59659;
    pub const CDV_77: u16 = 59660;
    pub const CD_78_PAL: u16 = 59661;
    pub const CD_78_LOG: u16 = 59662;
    pub const CDV_78: u16 = 59663;
    pub const CDV_79: u16 = 59664;
    pub const CDV_80: u16 = 59665;
    pub const CDV_81: u16 = 59666;
    pub const CDV_82: u16 = 59667;
    pub const CDV_83: u16 = 59668;
    pub const CDV_84: u16 = 59669;
    pub const CDV_85: u16 = 59670;
    pub const CDV_86: u16 = 59671;
    pub const CDV_87: u16 = 59672;
    pub const CD_100: u16 = 60087;
    pub const CD_101_LOG: u16 = 60088;
    pub const CD_101: u16 = 60099;
    pub const CD_102_LOG: u16 = 60090;
    pub const CD_102: u16 = 60091;
    pub const CD_103_PAL: u16 = 60092;
    pub const CD_103_LOG: u16 = 60093;
    pub const CD_103: u16 = 60094;
    pub const CD_104_PAL: u16 = 60095;
    pub const CD_104_LOG: u16 = 60096;
    pub const CD_104: u16 = 60097;
    pub const CD_105: u16 = 60098;
}
use files::*;

#[rustfmt::skip]
static MAIN_INTRO_SEQ: &[u16] = &[
        DELAY,       3000, // keep virgin screen up
        FADEDOWN,
        SHOWSCREEN, 60112, // revo screen + palette
        FADEUP,     60113,
        DELAY,       8000,
        FADEDOWN,
        SHOWSCREEN, 60114, // gibbo screen + palette
        FADEUP,     60115,
        DELAY,       2000,
        FADEDOWN,
        SEQEND
];

#[rustfmt::skip]
static CD_INTRO_SEQ: &[u16] = &[
        /* black screen */
        PLAYVOICE,  CDV_00, // Foster: "The old man was trying to tell the future. Looking for pictures in the campfire..."
        LOADBG,     59499,  // Fire crackle
        LOOPBG,
        WAITVOICE,
        PLAYVOICE,  CDV_01, // Shaman: "ohhh, I see evil..."
        /* Fade up shaman image while he says his line... */
        SHOWSCREEN, CD_1_LOG,
        FADEUP,     CD_PAL,
        /* And then play the animation showing the shadows of the fire on his face */
        BGFLIRT,    CD_1,
            WAITVOICE,
            PLAYVOICE,  CDV_02, // Shaman: "Evil born deep beneath the city... far from the light of day."
            WAITVOICE,
        STOPFLIRT,
        BGFLIRT,    CD_2,
            PLAYVOICE,  CDV_03, // Shaman: "I see it growing, safe beneath a sky of steel..."
            WAITVOICE,
            PLAYVOICE,  CDV_04, // Shaman: "Scheming in the dark... gathering strength."
        WAITFLIRT,
        WAITVOICE,
        PLAYVOICE,  CDV_05,     // Shaman: "And now... ohhh.... now the evil spreads..."
        DELAY,      2000,
        BGFLIRT,    CD_3,
            WAITVOICE,
            PLAYVOICE,  CDV_06, // Shaman: "It sends deadly feelers over the land above..."
        WAITFLIRT,
        WAITVOICE,
        PLAYVOICE,  CDV_07,     // Shaman: "Across the gap... reaching towards this very place!"
        BGFLIRT,    CD_5,
            WAITVOICE,
            PLAYVOICE,  CDV_08, // Foster: "I'd seen him do this a hundred times, but I humoured him."
            WAITVOICE,
            PLAYVOICE,  CDV_09, // Foster: "After all, he'd been like a father to me."
        WAITFLIRT,
        WAITVOICE,
        PLAYVOICE,  CDV_10,     // Foster: "And what does this evil want here?"
        BGFLIRT,    CD_7,
            WAITVOICE,
            PLAYVOICE,  CDV_11, // Shaman: "Oh, my son. I fear..."
        WAITFLIRT,
        FADEDOWN,
        SHOWSCREEN, CD_11_LOG,
        FADEUP,     CD_11_PAL,
        WAITVOICE,
        PLAYVOICE,  CDV_12,     // Shaman: "I fear the evil wants you!"
        DELAY,      1600,
        BGFLIRT,    CD_11,
            WAITVOICE,
            PLAYVOICE,  CDV_13, // Foster: "That was when Joey piped up..."
            WAITVOICE,
        WAITFLIRT,
        WAITVOICE,
        PLAYVOICE,  CDV_14,     // Joey: "Foster! Sensors detect incoming audio source!"
        LOADBG,     59498, // fire crackle to heli start
        PLAYBG,
        DOFLIRT,    CD_13,
        WAITVOICE,
        PLAYVOICE,  CDV_15,     // Shaman: "The evil! The evil is nearly here...!"
        FADEDOWN,
        SHOWSCREEN, CD_15_LOG,
        FADEUP,     CD_15_PAL,
        WAITVOICE,
        LOADBG,     59496, // quiet heli
        LOOPBG,
        PLAYVOICE,  CDV_16,     // Foster: "It sounded more like a 'copter than a demon."
        WAITVOICE,
        PLAYVOICE,  CDV_17,     // Foster: "But next thing, all hell let loose anyway..."
        DELAY,      2000,
        SHOWSCREEN, CD_17_LOG,
        WAITVOICE,
        BGFLIRT,    CD_17,
            PLAYVOICE,  CDV_18, // Shaman: "Run, Foster! Run! Hide from the evil!"
        LOADBG,     59497, // loud heli
        LOOPBG,
        WAITFLIRT,
        WAITVOICE,
        FADEDOWN,
        SHOWSCREEN | FULLSCREEN,    CD_19_LOG,
        FADEUP,     CD_19_PAL,
        PLAYVOICE,  CDV_19,     // Joey: "Foster! (zzzt) H-Help!"
        WAITVOICE,
        PLAYVOICE,  CDV_20,     // Joey: "Better make my next body move faster, Foster..."
        FADEDOWN,
        SHOWSCREEN | FULLSCREEN,    CD_20_LOG,
        FADEUP,     CD_19_PAL,
        WAITVOICE,
        LOADBG,     59496, // quiet heli
        LOOPBG,
        PLAYVOICE,  CDV_21,     // Foster: "He was only a robot, but, well, I loved the little guy."
        FADEDOWN,
        SHOWSCREEN | FULLSCREEN,    CD_21_LOG,
        FADEUP,     CD_19_PAL,
        WAITVOICE,
        PLAYVOICE,  CDV_22,     // Foster: "Then, as suddenly as it started, the shooting stopped."
        LOADBG,     59494, // heli whine
        PLAYBG,
        WAITVOICE,
        PLAYVOICE,  CDV_23,     // Foster: "There was a moment's silence as the copter cut its rotors, then..."
        /* fade down while Foster's saying his line */
        FADEDOWN,
        WAITVOICE,
        SHOWSCREEN | FULLSCREEN,    CD_24_LOG,
        FADEUP,     CD_23_PAL,
        PLAYVOICE,  CDV_24,     // Reich: "Whoever is in charge here, come forward..."
        WAITVOICE,
        PLAYVOICE,  CDV_25,     // Reich: "Now!!"
        WAITVOICE,
        PLAYVOICE,  CDV_26,     // Foster: "Only a fool would have argued with that firepower."
        WAITVOICE,
        FADEDOWN,
        SHOWSCREEN | FULLSCREEN,    CD_27_LOG,
        FADEUP,     CD_27_PAL,
        PLAYVOICE,  CDV_27,     // Shaman: "... I am the leader of these people... We are peaceful..."
        WAITVOICE,
        PLAYVOICE,  CDV_29,     // Reich: "Bring him here."
        WAITVOICE,
        PLAYVOICE,  CDV_30,     // Guard: "At once, Commander Reich."
        WAITVOICE,
        BGFLIRT,    CD_27,
            PLAYVOICE,  CDV_31, // Reich: "We're looking for someone..."
            WAITFLIRT,
            CLEARBOTTOM,
            WAITVOICE,
            PLAYVOICE,  CDV_32, // Reich: "Someone who doesn't belong here..."
            WAITVOICE,
            PLAYVOICE,  CDV_33, // Reich: "Who wasn't born in this garbage dump..."
            WAITVOICE,
            PLAYVOICE,  CDV_34, // Reich: "Who came from the city as a child..."
        WAITVOICE,
        PLAYVOICE,  CDV_35,     // Reich: "We want to take him home again."
        WAITVOICE,
        PLAYVOICE,  CDV_36,     // Foster: "My mind racing, I remembered where I'd seen that symbol before..."
            FADEDOWN,
            SHOWSCREEN | FULLSCREEN,    CD_35_LOG,
            FADEUP,     CD_35_PAL,
        WAITVOICE,
        PLAYVOICE,  CDV_37,     // Foster: "It was the day the tribe found me..."
            DOFLIRT,    CD_35,
        CLEARBOTTOM,
        WAITVOICE,
        PLAYVOICE,  CDV_38,     // Foster: "The day of the crash..."
            DOFLIRT,    CD_37,
        WAITVOICE,
        PLAYVOICE,  CDV_39,     // Foster: "The day my mother died."
        WAITVOICE,
        FADEDOWN,
        SHOWSCREEN | FULLSCREEN,    CD_40_LOG,
        FADEUP,     CD_40_PAL,
        PLAYVOICE,  CDV_40,     // Shaman: "You alright, city boy?"
        WAITVOICE,
        PLAYVOICE,  CDV_41,     // Shaman: "Got a name, son?"
        WAITVOICE,
        PLAYVOICE,  CDV_42,     // Foster: "R-Robert."
        WAITVOICE,
        FADEDOWN,
        SHOWSCREEN | FULLSCREEN,    CD_43_LOG,
        FADEUP,     CD_43_PAL,
        PLAYVOICE,  CDV_43,     // Shaman: "Hah! Welcome to the Gap, Robert!"
        WAITVOICE,
        DOFLIRT,    CD_43,
        PLAYVOICE,  CDV_45,     // Foster: "As he patched me up, the old man had gently explained that there was no way back into the City..."
        FADEDOWN,
        SHOWSCREEN, CD_45_LOG,
        FADEUP,     CD_45_PAL,
        WAITVOICE,
        PLAYVOICE,  CDV_46,     // Foster: "And I already knew there was nothing he could do for mother."
        DOFLIRT,    CD_45,
        WAITVOICE,
        FADEDOWN,
        SHOWSCREEN | FULLSCREEN,    CD_47_LOG,
        FADEUP,     CD_47_PAL,
        PLAYVOICE,  CDV_47,     // Foster: "His tribe was poor, but they treated me like one of their own..."
        WAITVOICE,
        PLAYVOICE,  CDV_48,     // Foster: "I learned how to survive in the wasteland they called the Gap..."
        FADEDOWN,
        SHOWSCREEN | FULLSCREEN,    CD_48_LOG,
        FADEUP,     CD_48_PAL,
        WAITVOICE,
        BGFLIRT,    CD_48,
            PLAYVOICE,  CDV_49, // Foster: "And scavenging from the City dumps."
            WAITVOICE,
            PLAYVOICE,  CDV_50, // Foster: "As the years passed, I forgot my life in the City."
        WAITFLIRT,
        WAITVOICE,
        PLAYVOICE,  CDV_51,     // Foster: "Discovered new talents..."
        BGFLIRT,    CD_49,
            WAITVOICE,
            PLAYVOICE,  CDV_52, // Foster: "Hah!"
            WAITVOICE,
            PLAYVOICE,  CDV_53, // Joey: "I'm your (zzzt) friend... call me (zzzt) Joey."
            WAITVOICE,
        WAITFLIRT,
        PLAYVOICE,  CDV_54,     // Foster: "And got a second name."
        DOFLIRT,    CD_50,
        WAITVOICE,
        PLAYVOICE,  CDV_55,     // Shaman: "This is what we'll call you now you've come of age, son."
        WAITVOICE,
        PLAYVOICE,  CDV_56,     // Shaman: "We found you... we fostered you..."
            FADEDOWN,
            SHOWSCREEN, CD_55_LOG,
            FADEUP,     CD_55_PAL,
        WAITVOICE,
        PLAYVOICE,  CDV_57,     // Shaman: "So that makes you Robert Foster."
        WAITVOICE,
        FADEDOWN,
        SHOWSCREEN, CD_58_LOG,
        FADEUP,     CD_58_PAL,
        PLAYVOICE,  CDV_58,     // Reich: "...Wasted enough time!"
        WAITVOICE,
        PLAYVOICE,  CDV_59,     // Reich: "Give us the runaway or we'll shoot everyone..."
        WAITVOICE,
        PLAYVOICE,  CDV_60,     // Reich: "Starting with you, grandad!"
        WAITVOICE,
        PLAYVOICE,  CDV_61,     // Foster: "The old man had been right, for once..."
        WAITVOICE,
        PLAYVOICE,  CDV_62,     // Foster: "It was me they wanted."
        BGFLIRT,    CD_58,
            WAITVOICE,
            PLAYVOICE,  CDV_63, // Shaman: "No, my son! Don't let the evil take you! Run!"
            WAITVOICE,
            PLAYVOICE,  CDV_64, // Guard: "DNA scan confirms it's him, sir."
        WAITFLIRT,
        WAITVOICE,
        PLAYVOICE,  CDV_65,     // Foster: "Evil had come to the Gap, just as he said."
        FADEDOWN,
        WAITVOICE,
        SHOWSCREEN, CD_66_LOG,
        FADEUP,     CD_66_PAL,
        PLAYVOICE,  CDV_66,     // Reich: "Take him."
        WAITVOICE,
        PLAYVOICE,  CDV_67,     // Foster: "But had the old man seen why it wanted me?"
            FADEDOWN,
            SHOWSCREEN, CD_67_LOG,
            FADEUP,     CD_67_PAL,
        WAITVOICE,
        PLAYVOICE,  CDV_68,     // Foster: "Or what it would do next?"
        WAITVOICE,
        PLAYVOICE,  CDV_69,     // Foster: "It was too late to ask him now."
            FADEDOWN,
            SHOWSCREEN, CD_69_LOG,
            FADEUP,     CD_69_PAL,
        WAITVOICE,
        PLAYVOICE,  CDV_70,     // Guard: "Leaving destruction zone, Commander Reich."
        DOFLIRT,    CD_69,
        WAITVOICE,
        FADEDOWN,
        PLAYVOICE,  CDV_71,     // Reich: "Good. Detonate."
        WAITVOICE,
        SHOWSCREEN | FULLSCREEN,    CD_72_LOG,
        FADEUP,     CD_72_PAL,
        PLAYVOICE,  CDV_72,     // Foster: "Much too late."
        WAITVOICE,
        FADEDOWN,
        SHOWSCREEN | FULLSCREEN,    CD_73_LOG,
        FADEUP,     CD_73_PAL,
        PLAYVOICE,  CDV_73,     // Foster: "Why, you murdering..."
        WAITVOICE,
        PLAYVOICE,  CDV_74,     // Reich: "Keep him quiet."
        WAITVOICE,
        PLAYVOICE,  CDV_75,     // Foster: "All I could do was wait."
        FADEDOWN,
        SHOWSCREEN | FULLSCREEN,    CD_76_LOG,
        FADEUP,     CD_76_PAL,
        WAITVOICE,
        PLAYVOICE,  CDV_76,     // Foster: "Just like on a hunt. Just like the old man taught me."
        WAITVOICE,
        PLAYVOICE,  CDV_77,     // Foster: "Wait... and be ready."
        WAITVOICE,
        FADEDOWN,
        CLEARBOTTOM,
        SHOWSCREEN, CD_78_LOG,
        FADEUP,     CD_78_PAL,
        PLAYVOICE,  CDV_78,     // Foster: "It was dawn when we reached the City."
        WAITVOICE,
        PLAYVOICE,  CDV_79,     // Reich: "Land in the central Security compound."
        WAITVOICE,
        PLAYVOICE,  CDV_80,     // Foster: "A dawn my tribe would never see."
        BGFLIRT,    CD_100,
            WAITVOICE,
            PLAYVOICE,  CDV_81, // Foster: "They were no more than a note in Reich's book now."
            WAITVOICE,
            PLAYVOICE,  CDV_82, // Guard: "Yes, sir. Locking on automatic landing beacon."
            WAITVOICE,
        WAITFLIRT,
        SHOWSCREEN, CD_101_LOG,
        BGFLIRT,    CD_101,
            PLAYVOICE,  CDV_83, // Foster: "But what was I? Why did..."
            WAITVOICE,
            PLAYVOICE,  CDV_84, // Guard: "Sir! The guidance system! It's gone crazy!"
            WAITVOICE,
            PLAYVOICE,  CDV_85, // Guard: "We're going to HIT!"
            WAITVOICE,
        WAITFLIRT,
        CLEARBOTTOM,
        SHOWSCREEN, CD_102_LOG,
        PLAYVOICE,  CDV_86,     // Foster: "Maybe I'd get some answers now."
        DOFLIRT,    CD_102,
        FADEDOWN,
        // This one could be fullscreen, but that causes animation glitches.
        SHOWSCREEN, CD_103_LOG,
        FADEUP,     CD_103_PAL,
        BGFLIRT,    CD_103,
        WAITVOICE,
        PLAYVOICE,  CDV_87,     // Foster: "If I survived another 'copter crash."
        WAITFLIRT,
        WAITVOICE,
        STARTMUSIC, 2,
        FADEDOWN,
        SHOWSCREEN | FULLSCREEN,    CD_104_LOG,
        FADEUP,     CD_104_PAL,
        DOFLIRT,    CD_104,
        DOFLIRT,    CD_105,
        SEQEND
];

#[rustfmt::skip]
static FLOPPY_INTRO_SEQ: &[u16] = &[
        // This one could be fullscreen, but that causes animation glitches.
        SHOWSCREEN,   60081,
        FADEUP,       60080,
        DOFLIRT,      60082,
        DOFLIRT,      60083,
        DOFLIRT,      60084, // Beneath a Steel Sky
        DOFLIRT,      60085,
        DOFLIRT,      60086,
        SCROLLFLIRT,
        COMMANDFLIRT, 60087, // => command list 4a
            136, IC_MAKE_SOUND,  1, 70,
             90, IC_FX_VOLUME,  80,
             50, IC_FX_VOLUME,  90,
              5, IC_FX_VOLUME, 100,
        COMMANDEND,
        SHOWSCREEN,   60088,
        COMMANDFLIRT, 60089, // => command list 4b (cockpit)
            1000, IC_PREPARE_TEXT,  77,
             220, IC_SHOW_TEXT,     20, 160, // radar detects jamming signal
             105, IC_REMOVE_TEXT,
             105, IC_PREPARE_TEXT,  81,
             105, IC_SHOW_TEXT,    170,  86, // well switch to override you fool
              35, IC_REMOVE_TEXT,
              35, IC_PREPARE_TEXT, 477,
              35, IC_SHOW_TEXT,     30, 160,
               3, IC_REMOVE_TEXT,
        COMMANDEND,
        CLEARBOTTOM,
        SHOWSCREEN,   60090,
        COMMANDFLIRT, 60091, // => command list 4c
            1000, IC_FX_VOLUME, 100,
              25, IC_FX_VOLUME, 110,
              15, IC_FX_VOLUME, 120,
               4, IC_FX_VOLUME, 127,
        COMMANDEND,
        FADEDOWN,
        // This one could be fullscreen, but that causes animation glitches.
        SHOWSCREEN,  60093,
        FADEUP,       60092,
        COMMANDFLIRT, 60094, // => command list 5
            31, IC_MAKE_SOUND, 2, 127,
        COMMANDEND,
        WAITMUSIC,
        FADEDOWN,
        SHOWSCREEN | FULLSCREEN,   60096,
        STARTMUSIC,       2,
        FADEUP,       60095,
        COMMANDFLIRT, 60097, // => command list 6a
            1000, IC_PREPARE_TEXT, 478,
              13, IC_SHOW_TEXT,    175, 155,
        COMMANDEND,
        COMMANDFLIRT, 60098, // => command list 6b
            131, IC_REMOVE_TEXT,
            131, IC_PREPARE_TEXT, 479,
             74, IC_SHOW_TEXT,    175, 155,
             45, IC_REMOVE_TEXT,
             45, IC_PREPARE_TEXT, 162,
             44, IC_SHOW_TEXT,    175, 155,
              4, IC_REMOVE_TEXT,
        COMMANDEND,
        SEQEND
];

#[cfg(test)]
mod tests {
    use super::super::host::{Action, Headless, HostEvent};
    use super::*;
    use std::path::Path;

    /// The CD version's data; tests that need it pass when it is absent.
    const DATA: &str = "/home/david/store/code/asciiverse/games/beneath-a-typographic-sky/reference/bass-cd-1.2";

    /// A silent headless engine (Headless never asks for audio), with
    /// `events` queued for it to read, one per poll.
    fn sky(events: Vec<HostEvent>) -> Option<Sky> {
        let p = Path::new(DATA);
        if !p.join("sky.dsk").exists() {
            eprintln!("skipping: no game data at {DATA}");
            return None;
        }
        let mut host = Headless::new();
        host.events.extend(events);
        Some(Sky::new(p, Box::new(host)).expect("game data"))
    }

    /// Runs a few game cycles and returns the room (Logic's SCREEN).
    fn room_after_ticks(sky: &mut Sky) -> u32 {
        for _ in 0..20 {
            sky.tick();
            sky.delay(80);
        }
        sky.vars[SCREEN as usize]
    }

    #[test]
    fn cd_intro_plays_through_into_room_0() {
        let Some(mut sky) = sky(Vec::new()) else { return };
        assert!(!sky.audio.audio.available(), "tests must stay silent");
        sky.start(false);
        let t = sky.host.millis();
        // the voices alone last over four minutes
        assert!(t > 240_000 && t < 400_000, "intro took {t} ms");
        // played through: init_screen0, not restart_game, so not past_intro yet
        assert!(!sky.sys.past_intro);
        assert!(!sky.sequence_running());
        assert!(sky.screen.texts_shown.is_empty());
        assert_eq!(room_after_ticks(&mut sky), 0);
    }

    #[test]
    fn cd_intro_skips_cleanly() {
        // other keys are dropped by the intro; each poll takes about 20 ms,
        // so the Escape comes some 30 seconds in, during the shaman scenes
        let mut events = vec![HostEvent::Key; 1500];
        events.push(HostEvent::Action(Action::Skip));
        let Some(mut sky) = sky(events) else { return };
        sky.start(false);
        let t = sky.host.millis();
        assert!(t > 20_000 && t < 60_000, "skipped at {t} ms");
        // skipped: restart_game took us to the start of the game
        assert!(sky.sys.past_intro);
        assert!(!sky.sequence_running());
        assert_eq!(sky.action, Action::None);
        assert_eq!(room_after_ticks(&mut sky), 0);
    }

    #[test]
    fn floppy_intro_plays_through_and_leaves_no_text() {
        let Some(mut sky) = sky(Vec::new()) else { return };
        assert!(sky.do_intro_alt(true));
        assert!(sky.screen.texts_shown.is_empty());
        assert!(!sky.sequence_running());
    }

    #[test]
    fn floppy_intro_text_is_type() {
        let Some(mut sky) = sky(Vec::new()) else { return };
        let mut it = Intro::default();
        sky.show_screen_file(60088, false);
        // what IC_PREPARE_TEXT 77 and IC_SHOW_TEXT 20, 160 do in list 4b
        let text = sky.get_text(77);
        let t = sky.display_text(text, Align::Center, INTRO_TEXT_WIDTH, 255);
        let rd = |i: usize| u16::from_le_bytes([t.data[i * 2], t.data[i * 2 + 1]]);
        it.text_buf = Some(TextBuf { width: rd(3), height: rd(4), data: t.data, lines: t.lines, x: 20, y: 160 });
        let before = sky.screen.current.clone();
        sky.screen.text_pixels = false;
        sky.show_text_buf(&mut it);
        assert_eq!(sky.screen.current, before, "no pixels when the terminal sets the type");
        assert_eq!(sky.screen.texts_shown.len(), 1);
        let o = &sky.screen.texts_shown[0];
        assert_eq!((o.x, o.y, o.width, o.color), (20, 160, 128, 255));
        assert!(o.lines.join(" ").contains("JAMMING"), "{:?}", o.lines);
        sky.restore_screen(&mut it);
        assert!(sky.screen.texts_shown.is_empty());
        assert_eq!(sky.screen.current, before);
    }
}
