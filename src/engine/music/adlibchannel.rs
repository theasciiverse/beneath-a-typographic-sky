//! One music channel of the AdLib driver. Port of ScummVM's
//! engines/sky/music/adlibchannel.cpp and adlibchannel.h.
//!
//! The channel reads its event stream, instrument table, frequency table
//! and register tables straight out of the section's music data file,
//! which holds the original AdLib driver as well as the tunes. As in
//! ScummVM, the register mirror also lives in that file and is shared by
//! all channels.
//!
//! Where the C++ code holds pointers into the music data, this keeps
//! offsets; every read is bounds checked and reads 0 past the end, where
//! the original would have read whatever followed in memory.

use super::opl::Opl;

/// Size of one `InstrumentStruct`.
const INSTRUMENT_SIZE: usize = 16;

// Field offsets inside `InstrumentStruct`.
const AD_OP1: usize = 0;
const AD_OP2: usize = 1;
const SR_OP1: usize = 2;
const SR_OP2: usize = 3;
const AMP_MOD_OP1: usize = 4;
const AMP_MOD_OP2: usize = 5;
const WAVE_SELECT_OP1: usize = 6;
const WAVE_SELECT_OP2: usize = 7;
const BINDED_EFFECT: usize = 8;
const FEED_BACK: usize = 9;
const TOT_OUT_LEV_OP1: usize = 10;
const TOT_OUT_LEV_OP2: usize = 11;
const SCALING_LEVEL: usize = 12;

pub(super) fn rd(data: &[u8], at: usize) -> u8 {
    data.get(at).copied().unwrap_or(0)
}

pub(super) fn rd16(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([rd(data, at), rd(data, at + 1)])
}

/// `AdLibChannel` with its `AdLibChannelType`.
pub struct AdLibChannel {
    event_data_ptr: u16,
    next_event_time: i32,
    loop_point: u16,
    adlib_channel_number: u8,
    last_command: u8,
    channel_active: bool,
    note: u8,
    adlib_reg1: u8,
    adlib_reg2: u8,
    /// Offset of the current `InstrumentStruct` in the music data; `None`
    /// is the NULL `instrumentData` pointer.
    instrument_data: Option<usize>,
    assigned_instrument: u8,
    channel_volume: u8,
    tremo_vibro: u8,
    freq_offset: u8,
    frequency: u16,

    music_volume: u16,
    /// Times the stream looped, for the log (not in ScummVM).
    pub(super) loops: u32,

    // offsets into the music data
    instruments: usize,
    frequence_table: usize,
    instrument_map: usize,
    register_table: usize,
    op_output_table: usize,
    adlib_reg_mirror: usize,
}

impl AdLibChannel {
    pub fn new(music_data: &[u8], start_of_data: u16, game_version: u32) -> AdLibChannel {
        let instrument_data_loc;
        let (frequence_table, register_table, op_output_table, adlib_reg_mirror);
        if game_version == 109 {
            //instrumentDataLoc = (_musicData[0x11D0] << 8) | _musicData[0x11CF];
            //_frequenceTable = (uint16 *)(_musicData + 0x835);
            //_registerTable = _musicData + 0xE35;
            //_opOutputTable = _musicData + 0xE47;
            //_adlibRegMirror = _musicData + 0xF4A;

            instrument_data_loc = rd16(music_data, 0x1204) as usize;
            frequence_table = 0x868;
            register_table = 0xE68;
            op_output_table = 0xE7A;
            adlib_reg_mirror = 0xF7D;
        } else if game_version == 267 {
            instrument_data_loc = rd16(music_data, 0x11FB) as usize;
            frequence_table = 0x7F4;
            register_table = 0xDF4;
            op_output_table = 0xE06;
            adlib_reg_mirror = 0xF55;
        } else {
            instrument_data_loc = rd16(music_data, 0x1205) as usize;
            frequence_table = 0x7FE;
            register_table = 0xDFE;
            op_output_table = 0xE10;
            adlib_reg_mirror = 0xF5F;
        }

        let mut ch = AdLibChannel {
            event_data_ptr: start_of_data,
            next_event_time: 0,
            loop_point: start_of_data,
            adlib_channel_number: 0,
            last_command: 0,
            channel_active: true,
            note: 0,
            adlib_reg1: 0,
            adlib_reg2: 0,
            instrument_data: None,
            assigned_instrument: 0xFF,
            channel_volume: 0x7F,
            tremo_vibro: 0,
            freq_offset: 0,
            frequency: 0,
            music_volume: 128,
            loops: 0,
            instruments: instrument_data_loc + 0x80,
            frequence_table,
            instrument_map: instrument_data_loc,
            register_table,
            op_output_table,
            adlib_reg_mirror,
        };
        ch.next_event_time = ch.get_next_event_time(music_data);
        ch
    }

    pub fn is_active(&self) -> bool {
        self.channel_active
    }

    pub fn update_volume(&mut self, volume: u16) {
        self.music_volume = volume;
    }

    /// Reads a field of the current instrument (0 when there is none, where
    /// the original would dereference NULL).
    fn ins(&self, data: &[u8], field: usize) -> u8 {
        match self.instrument_data {
            Some(at) => rd(data, at + field),
            None => 0,
        }
    }

    /// This class uses the same area for the register mirror as the original
    /// asm driver did (_musicData[0xF5F..0x105E]), so the cache is indeed shared
    /// by all instances of the class.
    fn set_register(&self, data: &mut [u8], opl: &mut Opl, reg_num: u8, value: u8) {
        let at = self.adlib_reg_mirror + reg_num as usize;
        if rd(data, at) != value {
            opl.write_reg(reg_num, value);
            if let Some(m) = data.get_mut(at) {
                *m = value;
            }
        }
    }

    /// `stopNote`; ScummVM also calls it from the destructor, which here
    /// is `MusicBase::stopMusicInternal` calling it before dropping the channel.
    pub fn stop_note(&mut self, data: &mut [u8], opl: &mut Opl) {
        if self.note & 0x20 != 0 {
            self.note &= !0x20;
            self.set_register(data, opl, 0xB0 | self.adlib_channel_number, self.note);
        }
    }

    fn get_next_event_time(&mut self, data: &[u8]) -> i32 {
        let mut ret_v: i32 = 0;
        let mut l_val = 0u8;
        for _ in 0..4 {
            l_val = rd(data, self.event_data_ptr as usize);
            self.event_data_ptr = self.event_data_ptr.wrapping_add(1);
            ret_v = (ret_v << 7) | (l_val & 0x7F) as i32;
            if l_val & 0x80 == 0 {
                break;
            }
        }
        if l_val & 0x80 != 0 {
            -1 // should never happen
        } else {
            ret_v
        }
    }

    fn next_byte(&mut self, data: &[u8]) -> u8 {
        let b = rd(data, self.event_data_ptr as usize);
        self.event_data_ptr = self.event_data_ptr.wrapping_add(1);
        b
    }

    /// Runs the channel `akt_time` ticks on; returns a new tempo, or 0.
    pub fn process(&mut self, akt_time: u16, data: &mut [u8], opl: &mut Opl) -> u8 {
        if !self.channel_active {
            return 0;
        }

        let mut return_val = 0;

        self.next_event_time -= akt_time as i32;
        while self.next_event_time < 0 && self.channel_active {
            let opcode = self.next_byte(data);
            if opcode & 0x80 != 0 {
                if opcode == 0xFF {
                    // dummy opcode
                } else if opcode >= 0x90 {
                    match opcode & 0xF {
                        0 => self.com90_case_note_off(data, opl),
                        1 => self.com90_stop_channel(data, opl),
                        2 => self.com90_setup_instrument(data, opl),
                        3 => return_val = self.com90_update_tempo(data),
                        5 => self.com90_get_freq_offset(data, opl),
                        6 => self.com90_get_channel_volume(data),
                        7 => self.com90_get_tremo_vibro(data),
                        8 => self.com90_loop_music(),
                        9 => self.com90_key_off(data, opl),
                        12 => self.com90_set_loop_point(),
                        // ScummVM stops the game here with error()
                        _ => log::error!("AdLibChannel: Unknown music opcode 0x{opcode:02X}"),
                    }
                } else {
                    // new adlib channel assignment
                    self.adlib_channel_number = opcode & 0xF;
                    self.adlib_reg1 = rd(data, self.register_table + (((opcode & 0xF) as usize) << 1));
                    self.adlib_reg2 = rd(data, self.register_table + ((((opcode & 0xF) as usize) << 1) | 1));
                }
            } else {
                self.last_command = opcode;
                self.stop_note(data, opl);
                // not sure why this "if" is necessary...either a bug in my
                // code or a bug in the music data (section 1, music 2)
                if self.instrument_data.is_some() || self.tremo_vibro != 0 {
                    self.setup_instrument(opcode, data, opl);

                    let volume = self.next_byte(data);
                    self.setup_channel_volume(volume, data, opl);
                } else {
                    self.event_data_ptr = self.event_data_ptr.wrapping_add(1);
                }
            }
            if self.channel_active {
                self.next_event_time += self.get_next_event_time(data);
            }
        }
        return_val
    }

    fn setup_instrument(&mut self, opcode: u8, data: &mut [u8], opl: &mut Opl) {
        let next_note;
        if self.tremo_vibro != 0 {
            let new_instrument = rd(data, self.instrument_map + opcode as usize);
            if new_instrument != self.assigned_instrument {
                self.assigned_instrument = new_instrument;
                self.instrument_data = Some(self.instruments + new_instrument as usize * INSTRUMENT_SIZE);
                self.adlib_setup_instrument(data, opl);
            }
            self.last_command = self.ins(data, BINDED_EFFECT);
            next_note = self.get_next_note(self.last_command, data);
        } else {
            let param = opcode.wrapping_sub(0x18).wrapping_add(self.ins(data, BINDED_EFFECT));
            next_note = self.get_next_note(param, data);
        }
        self.frequency = next_note;
        let ch = self.adlib_channel_number;
        self.set_register(data, opl, 0xA0 | ch, next_note as u8);
        self.set_register(data, opl, 0xB0 | ch, ((next_note >> 8) as u8) | 0x20);
        self.note = ((next_note >> 8) as u8) | 0x20;
    }

    fn setup_channel_volume(&mut self, volume: u8, data: &mut [u8], opl: &mut Opl) {
        let mut res_vol: u32 = ((volume as u32 + 1) * (self.ins(data, TOT_OUT_LEV_OP2) as u32 + 1)) << 1;
        res_vol &= 0xFFFF;
        res_vol *= (self.channel_volume as u32 + 1) << 1;
        res_vol >>= 8;
        res_vol *= (self.music_volume as u32) << 1;
        res_vol >>= 16;
        if res_vol >= 0x81 {
            // ScummVM: assert(resVol < 0x81)
            log::warn!("AdLibChannel: volume index {res_vol:#x} out of range");
        }
        let scaling = self.ins(data, SCALING_LEVEL) as u32;
        let result_op = (((scaling << 6) & 0xC0) as u8) | rd(data, self.op_output_table + res_vol as usize);
        self.set_register(data, opl, 0x40 | self.adlib_reg2, result_op);
        if self.ins(data, FEED_BACK) & 1 != 0 {
            res_vol = ((volume as u32 + 1) * (self.ins(data, TOT_OUT_LEV_OP1) as u32 + 1)) << 1;
            res_vol &= 0xFFFF;
            res_vol *= (self.channel_volume as u32 + 1) << 1;
            res_vol >>= 8;
            res_vol *= (self.music_volume as u32) << 1;
            res_vol >>= 16;
        } else {
            res_vol = self.ins(data, TOT_OUT_LEV_OP1) as u32;
        }
        if res_vol >= 0x81 {
            // ScummVM: assert(resVol < 0x81)
            log::warn!("AdLibChannel: volume index {res_vol:#x} out of range");
        }
        let result_op = (((scaling << 2) & 0xC0) as u8) | rd(data, self.op_output_table + res_vol as usize);
        self.set_register(data, opl, 0x40 | self.adlib_reg1, result_op);
    }

    fn adlib_setup_instrument(&mut self, data: &mut [u8], opl: &mut Opl) {
        let (r1, r2, ch) = (self.adlib_reg1, self.adlib_reg2, self.adlib_channel_number);
        let i = |f| self.ins(data, f);
        let writes = [
            (0x60 | r1, i(AD_OP1)),
            (0x60 | r2, i(AD_OP2)),
            (0x80 | r1, i(SR_OP1)),
            (0x80 | r2, i(SR_OP2)),
            (0xE0 | r1, i(WAVE_SELECT_OP1)),
            (0xE0 | r2, i(WAVE_SELECT_OP2)),
            (0xC0 | ch, i(FEED_BACK)),
            (0x20 | r1, i(AMP_MOD_OP1)),
            (0x20 | r2, i(AMP_MOD_OP2)),
        ];
        for (reg, value) in writes {
            self.set_register(data, opl, reg, value);
        }
    }

    fn get_next_note(&self, param: u8, data: &[u8]) -> u16 {
        let mut freq_index: i16 = self.freq_offset as i16 - 0x40;
        if freq_index >= 0x3F {
            freq_index += 1;
        }
        freq_index *= 2;
        freq_index = freq_index.wrapping_add((param as i16) << 6);
        // C++ % and / truncate towards zero, as Rust's do; a negative
        // index reads in front of the table, as the original does
        let idx = (freq_index % 0x300) as isize;
        let at = self.frequence_table as isize + idx * 2;
        let freq_data = if at >= 0 { rd16(data, at as usize) } else { 0 };
        if freq_index % 0x300 >= 0x1C0 || freq_index / 0x300 > 0 {
            ((((freq_index / 0x300) as i32 - 1) << 10) + (freq_data & 0x7FF) as i32) as u16
        } else {
            // looks like a bug. dunno why. It's what the ASM code says.
            ((freq_data as i16) >> 1) as u16
        }
    }

    //- command 90h routines

    fn com90_case_note_off(&mut self, data: &mut [u8], opl: &mut Opl) {
        if rd(data, self.event_data_ptr as usize) == self.last_command {
            self.stop_note(data, opl);
        }
        self.event_data_ptr = self.event_data_ptr.wrapping_add(1);
    }

    fn com90_stop_channel(&mut self, data: &mut [u8], opl: &mut Opl) {
        self.stop_note(data, opl);
        self.channel_active = false;
    }

    fn com90_setup_instrument(&mut self, data: &mut [u8], opl: &mut Opl) {
        self.channel_volume = 0x7F;
        self.freq_offset = 0x40;
        self.assigned_instrument = self.next_byte(data);
        self.instrument_data = Some(self.instruments + self.assigned_instrument as usize * INSTRUMENT_SIZE);
        self.adlib_setup_instrument(data, opl);
    }

    fn com90_update_tempo(&mut self, data: &[u8]) -> u8 {
        self.next_byte(data)
    }

    fn com90_get_freq_offset(&mut self, data: &mut [u8], opl: &mut Opl) {
        self.freq_offset = self.next_byte(data);
        if self.note & 0x20 != 0 {
            let param = self.last_command.wrapping_sub(0x18).wrapping_add(self.ins(data, BINDED_EFFECT));
            let next_note = self.get_next_note(param, data);
            let ch = self.adlib_channel_number;
            self.set_register(data, opl, 0xA0 | ch, next_note as u8);
            self.set_register(data, opl, 0xB0 | ch, ((next_note >> 8) as u8) | 0x20);
            self.note = ((next_note >> 8) as u8) | 0x20;
        }
    }

    fn com90_get_channel_volume(&mut self, data: &[u8]) {
        self.channel_volume = self.next_byte(data);
    }

    fn com90_get_tremo_vibro(&mut self, data: &[u8]) {
        self.tremo_vibro = self.next_byte(data);
    }

    fn com90_loop_music(&mut self) {
        self.loops += 1;
        self.event_data_ptr = self.loop_point;
    }

    fn com90_key_off(&mut self, data: &mut [u8], opl: &mut Opl) {
        self.stop_note(data, opl);
    }

    fn com90_set_loop_point(&mut self) {
        self.loop_point = self.event_data_ptr;
    }
}
