//! Section loading, song changes and tempo. Port of ScummVM's
//! engines/sky/music/musicbase.cpp and musicbase.h, as methods of
//! `AdLibMusic` (the only driver ported).
//!
//! Not ported: the digital tracks of the Music Enhancement Project
//! (`music_SNN` files that ScummVM prefers over the MIDI music when they
//! are present); this plays the original AdLib music only. ScummVM's
//! mutex is the `Mutex` around the whole driver in `mod.rs`.

use super::adlibchannel::{rd, rd16};
use super::adlibmusic::AdLibMusic;

pub const FILES_PER_SECTION: u16 = 4;

/// The body of `updateTempo`: song time units per 50 Hz timer tick, 16.16.
fn tempo(music_tempo0: u8, music_tempo1: u8) -> u32 {
    let tempo_mul = (music_tempo0 as u16).wrapping_mul(music_tempo1 as u16) as u32;
    let divisor = (0x4446390 / 23864) as u16 as u32;
    let mut tempo = (tempo_mul / divisor) << 16;
    tempo |= (((tempo_mul % divisor) << 16) | (tempo_mul / divisor)) / divisor;
    tempo
}

impl AdLibMusic {
    /// The file `load_section` needs for `section`.
    pub fn section_file(&self, section: u8) -> u16 {
        self.driver_file_base + FILES_PER_SECTION * section as u16
    }

    /// `loadSection`. `music_data` is the loaded `section_file(section)`;
    /// loading it is left to the caller so the audio thread is not held
    /// up by the disk.
    pub fn load_section(&mut self, section: u8, music_data: Option<Vec<u8>>, game_version: u32) {
        if self.current_music != 0 {
            self.stop_music_internal();
        }
        if music_data.is_none() {
            // ScummVM would crash on the missing file here
            log::error!("music data for section {section} not found");
        }
        self.music_data = music_data;
        self.game_version = game_version;
        self.current_section = section;

        self.music_tempo0 = 0x78; // init constants taken from idb file, area ~0x1060
        self.music_tempo1 = 0xC0;
        self.on_next_poll = 0;
        self.tempo = 0x10001;
        self.akt_time = 0x10001;
        self.channels.clear();
        self.current_music = 0;
        self.setup_pointers();
        self.start_driver();
    }

    pub fn music_is_playing(&self) -> bool {
        self.channels.iter().any(|c| c.is_active())
    }

    /// `stopMusic`. As in ScummVM this does not clear the current music, so
    /// a following `start_music` of the same number does not restart it;
    /// the game stops music with `start_music(0)`.
    pub fn stop_music(&mut self) {
        self.stop_music_internal();
    }

    pub(super) fn stop_music_internal(&mut self) {
        // ~AdLibChannel calls stopNote
        let mut channels = std::mem::take(&mut self.channels);
        if let Some(d) = self.music_data.as_deref_mut() {
            for ch in &mut channels {
                ch.stop_note(d, &mut self.opl);
            }
        }
    }

    fn update_tempo(&mut self) {
        self.tempo = tempo(self.music_tempo0, self.music_tempo1);
    }

    fn load_new_music(&mut self) {
        let Some(d) = self.music_data.as_deref() else { return };
        let loc = self.music_data_loc as usize;
        if self.on_next_poll > rd(d, loc) {
            // ScummVM stops the game here with error()
            log::error!("Music {} requested but doesn't exist in file.", self.on_next_poll);
            self.on_next_poll = self.current_music;
            return;
        }
        if self.current_music != 0 {
            self.stop_music_internal();
        }

        self.current_music = self.on_next_poll;
        log::debug!("music tick {}: song {} of section {}", self.ticks, self.current_music, self.current_section);

        if self.current_music == 0 {
            return;
        }

        // (ScummVM tries the Music Enhancement Project's digital audio here)

        // no digital audio, resort to MIDI playback
        let Some(d) = self.music_data.as_deref() else { return };
        let mut music_pos = rd16(d, loc + 1);
        music_pos = music_pos.wrapping_add(self.music_data_loc).wrapping_add(((self.current_music - 1) as u16) << 1);
        music_pos = rd16(d, music_pos as usize).wrapping_add(self.music_data_loc);
        let music_pos = music_pos as usize;

        self.music_tempo0 = rd(d, music_pos);
        self.music_tempo1 = rd(d, music_pos + 1);

        self.setup_channels(music_pos + 2);

        self.update_tempo();
    }

    pub(super) fn poll_music(&mut self) {
        self.ticks += 1;
        if self.on_next_poll != self.current_music {
            self.load_new_music();
        }

        self.akt_time = self.akt_time.wrapping_add(self.tempo);

        let Some(d) = self.music_data.as_deref_mut() else { return };
        for (n, ch) in self.channels.iter_mut().enumerate() {
            let (loops, active) = (ch.loops, ch.is_active());
            let new_tempo = ch.process((self.akt_time >> 16) as u16, d, &mut self.opl);
            if new_tempo != 0 {
                self.music_tempo1 = new_tempo;
                log::debug!("music tick {}: tempo {new_tempo:#x}", self.ticks);
                // updateTempo(), spelt out for the borrow of the channels
                self.tempo = tempo(self.music_tempo0, self.music_tempo1);
            }
            if ch.loops != loops {
                log::debug!("music tick {}: channel {n} loops", self.ticks);
            }
            if ch.is_active() != active {
                log::debug!("music tick {}: channel {n} stops", self.ticks);
            }
        }
        self.akt_time &= 0xFFFF;
    }

    pub fn start_music(&mut self, param: u16) {
        let song = (param & 0xF) as u8;
        self.on_next_poll = song;
        // (ScummVM loads the Music Enhancement Project's digital track here)
    }

    pub fn give_volume(&self) -> u8 {
        self.music_volume as u8
    }

    pub fn give_current_music(&self) -> u8 {
        self.current_music
    }

    pub fn current_section(&self) -> u8 {
        self.current_section
    }

    /// Number of songs in the loaded section (0 without one).
    pub fn song_count(&self) -> u8 {
        match self.music_data.as_deref() {
            Some(d) => rd(d, self.music_data_loc as usize),
            None => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tempo_matches_scummvm_arithmetic() {
        // the start tempo: 0x78 * 0xC0 / (0x4446390 / 23864 = 2999), the
        // fraction worked out the way updateTempo does it
        assert_eq!(super::tempo(0x78, 0xC0), 0x7AEBC);
        // the product is a uint16 in ScummVM
        assert_eq!(super::tempo(0xFF, 0xFF), (0xFE01 / 2999) << 16 | ((0xFE01 % 2999) << 16 | 0xFE01 / 2999) / 2999);
    }
}
