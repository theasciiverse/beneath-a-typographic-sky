//! The AdLib music driver. Port of ScummVM's engines/sky/music/adlibmusic.cpp
//! and adlibmusic.h, with `EmulatedChip::readBuffer` from audio/chip.cpp,
//! which is where ScummVM's OPL timer calls `AdLibMusic::onTimer`.
//!
//! ScummVM splits the driver into `MusicBase` (musicbase.cpp, ported in
//! `musicbase.rs` as a second `impl` block on this type) and the AdLib
//! subclass; only AdLib is ported, so the two are one struct here.

use super::adlibchannel::{rd, rd16, AdLibChannel};
use super::opl::{Opl, FIXP_SHIFT};

/// `AdLibMusic` together with its `MusicBase` state.
pub struct AdLibMusic {
    pub(super) opl: Opl,
    pub(super) music_data: Option<Vec<u8>>,
    pub(super) game_version: u32,

    pub(super) music_data_loc: u16,
    pub(super) driver_file_base: u16,

    pub(super) music_volume: u16,
    pub(super) current_music: u8,
    pub(super) current_section: u8,
    /// can be changed by music stream
    pub(super) music_tempo0: u8,
    /// given once per music
    pub(super) music_tempo1: u8,
    /// calculated from music_tempo0 and music_tempo1
    pub(super) tempo: u32,
    pub(super) akt_time: u32,
    /// `_onNextPoll.musicToProcess`
    pub(super) on_next_poll: u8,
    /// `_channels[0.._numberOfChannels]`
    pub(super) channels: Vec<AdLibChannel>,
    /// Offset of the OPL init sequence in the music data.
    init_sequence: usize,
    /// Timer ticks polled, for the log (not in ScummVM).
    pub(super) ticks: u64,
}

impl AdLibMusic {
    /// `AdLibMusic::AdLibMusic` with `MusicBase::MusicBase`: creates the
    /// chip and starts its 50 Hz timer. `rate` is the output sample rate.
    pub fn new(rate: u32) -> AdLibMusic {
        let mut opl = Opl::new(rate);
        // _opl->start(new Common::Functor0Mem<void, AdLibMusic>(this, &AdLibMusic::onTimer), 50);
        opl.set_callback_frequency(50);
        AdLibMusic {
            opl,
            music_data: None,
            game_version: 0,
            music_data_loc: 0,
            driver_file_base: 60202,
            music_volume: 127,
            current_music: 0,
            current_section: 0,
            music_tempo0: 0,
            music_tempo1: 0,
            tempo: 0,
            akt_time: 0,
            on_next_poll: 0,
            channels: Vec::new(),
            init_sequence: 0,
            ticks: 0,
        }
    }

    fn on_timer(&mut self) {
        if self.music_data.is_some() {
            self.poll_music();
        }
    }

    pub(super) fn setup_pointers(&mut self) {
        let Some(d) = self.music_data.as_deref() else { return };
        if self.game_version == 109 {
            // disk demo uses a different AdLib driver version, some offsets have changed
            //_musicDataLoc = (_musicData[0x11CC] << 8) | _musicData[0x11CB];
            //_initSequence = _musicData + 0xEC8;

            self.music_data_loc = rd16(d, 0x1200);
            self.init_sequence = 0xEFB;
        } else if self.game_version == 267 {
            self.music_data_loc = rd16(d, 0x11F7);
            self.init_sequence = 0xE87;
        } else {
            self.music_data_loc = rd16(d, 0x1201);
            self.init_sequence = 0xE91;
        }
    }

    /// `setupChannels`; `channel_data` is the offset of the channel count.
    pub(super) fn setup_channels(&mut self, channel_data: usize) {
        let Some(d) = self.music_data.as_deref() else { return };
        let number_of_channels = rd(d, channel_data) as usize;
        let channel_data = channel_data + 1;
        self.channels.clear();
        for cnt in 0..number_of_channels {
            let ch_data_start = rd16(d, channel_data + cnt * 2).wrapping_add(self.music_data_loc);
            let mut ch = AdLibChannel::new(d, ch_data_start, self.game_version);
            // also set proper volume - Fixes bug where intro music plays at full volume
            ch.update_volume(self.music_volume);
            self.channels.push(ch);
        }
    }

    pub(super) fn start_driver(&mut self) {
        let Some(d) = self.music_data.as_deref() else { return };
        let mut cnt = self.init_sequence;
        while rd(d, cnt) != 0 || rd(d, cnt + 1) != 0 {
            self.opl.write_reg(rd(d, cnt), rd(d, cnt + 1));
            cnt += 2;
        }
    }

    pub fn set_volume(&mut self, param: u16) {
        self.music_volume = param;
        for ch in &mut self.channels {
            ch.update_volume(self.music_volume);
        }
    }

    /// `EmulatedChip::readBuffer` for a mono chip: fills `buffer`, calling
    /// the 50 Hz timer (`onTimer`) at its place between the samples.
    pub fn read_buffer(&mut self, buffer: &mut [i16]) {
        let mut buffer = buffer;
        while !buffer.is_empty() {
            let step = (buffer.len() as i64).min(self.opl.next_tick >> FIXP_SHIFT) as usize;

            let (now, rest) = buffer.split_at_mut(step);
            self.opl.generate_samples(now);

            self.opl.next_tick -= (step as i64) << FIXP_SHIFT;
            if self.opl.next_tick >> FIXP_SHIFT == 0 {
                self.on_timer();
                self.opl.next_tick += self.opl.samples_per_tick;
            }

            buffer = rest;
        }
    }
}
