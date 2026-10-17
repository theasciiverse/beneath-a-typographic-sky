//! Sound effects and speech. Port of ScummVM's engines/sky/sound.cpp and
//! sound.h (GPL-3.0-or-later, like this crate), plus the few sound calls
//! intro.cpp makes on the mixer directly.
//!
//! Each section has one sound file (60203 + 4 * section) holding the
//! original Sound Blaster driver, a table of sample rates, a table of
//! effects and the 8-bit unsigned samples. Scripts start effects by a
//! number from 256 up (`fnStartFx`), which `MUSIC_LIST` turns into an
//! effect in the loaded section and a volume for the current room; some
//! start after a delay, counted down in game ticks by `check_fx_queue`.
//! On the CD version each line of dialogue is also a file of its own
//! (`start_speech`).
//!
//! ScummVM reads the room from `Logic::_scriptVariables[SCREEN]` and the
//! flags and game version from `SkyEngine::_systemVars`. Here they are
//! parameters, so `Sound` holds no global state.
//!
//! Sounds play through `crate::audio`. Without an audio device nothing is
//! heard and every sound counts as finished at once (see `start_speech`).

// Most of this is called by the engine loop, which is not written yet.
#![allow(dead_code)]

use std::sync::Arc;

use rodio::mixer::Mixer;

use super::defs::{SF_ALLOW_SPEECH, SF_FX_OFF, SF_GAME_RESTORED, SF_ROLAND, SF_SBLASTER};
use super::disk::{Disk, HEADER_SIZE};
use crate::audio::{self, Audio, Handle, Pcm};

/// Mixer ids ScummVM plays each kind of sound under; starting a sound on
/// an id stops what was playing there.
pub const SOUND_CH0: usize = 0;
pub const SOUND_CH1: usize = 1;
pub const SOUND_BG: usize = 2;
pub const SOUND_VOICE: usize = 3;
pub const SOUND_SPEECH: usize = 4;

const SOUND_FILE_BASE: u16 = 60203;
const MAX_FX_NUMBER: u32 = 393;
const SFXF_START_DELAY: u8 = 0x80;
const SFXF_SAVE: u8 = 0x20;
const MAX_QUEUED_FX: usize = 4;

/// ScummVM's `Audio::Mixer::kMaxChannelVolume`, the volume `SkyEngine`
/// creates `Sound` with.
pub const MAX_CHANNEL_VOLUME: u8 = 255;

/// ScummVM's default "sfx_volume" and "speech_volume" (sky.cpp), out of
/// `kMaxMixerVolume` (256).
pub const DEFAULT_TYPE_VOLUME: u16 = 192;
const MAX_MIXER_VOLUME: u16 = 256;

/// Text numbers to file numbers: the base for each section (the top four
/// bits of a text number), to which the line's number within the section
/// is added. Speech file = 50000 + that.
const SPEECH_CONVERT_TABLE: [u16; 8] = [
    0,                                  // Text numbers to file numbers
    600,                                //  553 lines in section 0
    600 + 500,                          //  488 lines in section 1
    600 + 500 + 1330,                   // 1303 lines in section 2
    600 + 500 + 1330 + 950,             //  922 lines in section 3
    600 + 500 + 1330 + 950 + 1150,      // 1140 lines in section 4
    600 + 500 + 1330 + 950 + 1150 + 550, //  531 lines in section 5
    600 + 500 + 1330 + 950 + 1150 + 550 + 150, // 150 lines in section 6
];

/// One delayed effect waiting to start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SfxQueue {
    /// Ticks left; 0 means the slot is free.
    pub count: u8,
    pub fx_no: u8,
    pub chan: u8,
    pub vol: u8,
}

/// Where an effect may play and how loud. Room 255 ends the list; a list
/// that starts with 255 means every room, at that entry's volume.
#[derive(Clone, Copy, Debug)]
struct RoomList {
    room: u8,
    adlib_volume: u8,
    roland_volume: u8,
}

/// An effect as scripts know it: which sound of the section's file it is,
/// its flags (`SFXF_SAVE`, or `SFXF_START_DELAY` plus a delay in ticks in
/// the low 7 bits) and the rooms it plays in.
#[derive(Debug)]
struct Sfx {
    sound_no: u8,
    flags: u8,
    room_list: [RoomList; 10],
}

const fn rl(room: u8, adlib_volume: u8, roland_volume: u8) -> RoomList {
    RoomList { room, adlib_volume, roland_volume }
}

/// The zeroed entries C leaves after a shorter initialiser.
const RL0: RoomList = rl(0, 0, 0);

/// One effect of the loaded section, ready to play.
#[derive(Clone)]
pub struct SfxSample {
    pub pcm: Pcm,
    /// The raw rate from the file's table, before ScummVM caps it at 11025.
    pub table_rate: u16,
}

/// Why `load_section` failed. ScummVM stops the engine with `error()` for
/// an unknown driver and would crash on a missing file.
#[derive(Debug)]
pub enum SectionError {
    Missing(u16),
    UnknownDriver(u16),
}

impl std::fmt::Display for SectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SectionError::Missing(n) => write!(f, "sound file {n} not found"),
            SectionError::UnknownDriver(n) => write!(f, "sound file {n}: unknown sounddriver version"),
        }
    }
}

pub struct Sound {
    mixer: Option<Mixer>,
    /// What plays under each mixer id (`SOUND_CH0` to `SOUND_SPEECH`), and
    /// at which channel volume (0 to 255).
    channels: [Option<Handle>; 5],
    channel_volumes: [u16; 5],

    /// The two effects to restart after a restore, as `sound_no | volume
    /// << 8`, 0xFFFF for none. Written to and read from save games
    /// (control.cpp) as two words.
    pub save_sounds: [u16; 2],
    /// Number of effects in the loaded section's file.
    pub sounds_total: u8,

    /// The loaded section's sound file (`_soundData`), shared with the
    /// effects playing from it.
    sound_data: Option<Arc<[u8]>>,
    /// Offsets into `sound_data`: `_sfxBaseOfs`, which is also where
    /// `_sfxInfo` points, and `_sampleRates`.
    sfx_base_ofs: u16,
    sample_rates_ofs: usize,
    main_sfx_volume: u8,
    is_paused: bool,
    /// `_sfxQueue`, static in ScummVM.
    sfx_queue: [SfxQueue; MAX_QUEUED_FX],

    /// ScummVM's per-sound-type volumes (`sfx_volume`, `speech_volume`),
    /// 0 to 256, applied on top of each sound's own volume.
    sfx_volume: u16,
    speech_volume: u16,
}

impl Sound {
    /// `Sound(mixer, disk, pVolume)`: `volume` is the main effects volume,
    /// which SkyEngine sets to `MAX_CHANNEL_VOLUME`. The disk is passed to
    /// the calls that load from it.
    pub fn new(audio: &Audio, volume: u8) -> Sound {
        Sound {
            mixer: audio.mixer(),
            channels: Default::default(),
            channel_volumes: [0; 5],
            save_sounds: [0xFFFF; 2],
            sounds_total: 0,
            sound_data: None,
            sfx_base_ofs: 0,
            sample_rates_ofs: 0,
            main_sfx_volume: volume,
            is_paused: false,
            sfx_queue: [SfxQueue::default(); MAX_QUEUED_FX],
            sfx_volume: DEFAULT_TYPE_VOLUME,
            speech_volume: DEFAULT_TYPE_VOLUME,
        }
    }

    /// The delayed effects waiting to start.
    pub fn sfx_queue(&self) -> &[SfxQueue; MAX_QUEUED_FX] {
        &self.sfx_queue
    }

    /// Sets ScummVM's "sfx_volume" (0 to 256), for effects and the intro's
    /// background sound, including those playing now.
    pub fn set_sfx_volume(&mut self, v: u16) {
        self.sfx_volume = v.min(MAX_MIXER_VOLUME);
        self.update_gains();
    }

    /// Sets ScummVM's "speech_volume" (0 to 256), for speech and the
    /// intro's voices.
    pub fn set_speech_volume(&mut self, v: u16) {
        self.speech_volume = v.min(MAX_MIXER_VOLUME);
        self.update_gains();
    }

    /// The sound type volume for mixer id `id`: speech for the speech and
    /// intro voice ids, effects for the rest.
    fn type_volume(&self, id: usize) -> u16 {
        if id == SOUND_SPEECH || id == SOUND_VOICE {
            self.speech_volume
        } else {
            self.sfx_volume
        }
    }

    fn update_gains(&self) {
        for (id, h) in self.channels.iter().enumerate() {
            if let Some(h) = h {
                h.set_gain(Self::gain(self.channel_volumes[id], self.type_volume(id)));
            }
        }
    }

    /// The gain for a sound of channel volume `vol` (0 to 255) and sound
    /// type volume `type_vol` (0 to 256), as ScummVM's mixer works it out
    /// (`Channel::updateChannelVolumes` and the rate converter): their
    /// product over 255 * 256.
    fn gain(vol: u16, type_vol: u16) -> f32 {
        (vol as f32 * type_vol as f32) / (255.0 * 256.0)
    }

    fn stop_id(&mut self, id: usize) {
        if let Some(h) = self.channels[id].take() {
            h.stop();
        }
    }

    /// `_mixer->stopAll()`: stops every sound `Sound` started. ScummVM's
    /// AdLib music runs on its own (an OPL timer), so music is not touched.
    pub fn stop_all(&mut self) {
        for id in 0..self.channels.len() {
            self.stop_id(id);
        }
    }

    /// Plays `pcm` under mixer id `id` at channel volume `vol` (0 to 255).
    /// `stop_first` is false only for the intro voice, which ScummVM starts
    /// without stopping that id.
    fn play_id(&mut self, id: usize, pcm: Pcm, vol: u16, stop_first: bool) {
        if stop_first {
            self.stop_id(id);
        }
        let gain = Self::gain(vol, self.type_volume(id));
        if let Some(m) = &self.mixer {
            self.channels[id] = Some(audio::play_on(m, pcm, gain));
            self.channel_volumes[id] = vol;
        }
    }

    fn is_active(&self, id: usize) -> bool {
        self.channels[id].as_ref().is_some_and(Handle::is_active)
    }

    /// `loadSection`: stops all effects, loads `section`'s sound file and
    /// finds its tables. Clears the fx queue unless `system_flags` has
    /// `SF_GAME_RESTORED`, in which case the queue holds the effects the
    /// restored game must restart. `game_version` matters only for the
    /// 0.0109 demo. Logic calls this with its section number plus one; the
    /// intro with 0.
    pub fn load_section(
        &mut self,
        disk: &Disk,
        section: u8,
        game_version: u32,
        system_flags: u32,
    ) -> Result<(), SectionError> {
        self.fn_stop_fx();
        self.stop_all();

        self.sound_data = None;
        let file = section as u16 * 4 + SOUND_FILE_BASE;
        let data: Arc<[u8]> = disk.load(file).ok_or(SectionError::Missing(file))?.into();
        let asm_ofs: usize = if game_version == 109 {
            if section == 0 {
                0x78
            } else {
                0x7C
            }
        } else {
            0x7E
        };

        // The tables are found through the original driver's code, so check
        // that it is the driver we know.
        let at = |i: usize| data.get(asm_ofs + i).copied();
        if at(0) != Some(0x3C)
            || at(0x27) != Some(0x8D)
            || at(0x28) != Some(0x1E)
            || at(0x2F) != Some(0x8D)
            || at(0x30) != Some(0x36)
        {
            return Err(SectionError::UnknownDriver(file));
        }
        let le16 = |i: usize| u16::from_le_bytes([data[asm_ofs + i], data[asm_ofs + i + 1]]);

        self.sounds_total = data[asm_ofs + 1];
        self.sample_rates_ofs = le16(0x29) as usize;
        self.sfx_base_ofs = le16(0x31);
        self.sound_data = Some(data);

        // if we just restored a savegame, the sfxqueue holds the sound we need to restart
        if system_flags & SF_GAME_RESTORED == 0 {
            for q in &mut self.sfx_queue {
                q.count = 0;
            }
        }
        Ok(())
    }

    /// The samples of effect `sound` (0 to `sounds_total`) of the loaded
    /// section, as `playSound` finds them. `None` without a section or if
    /// the tables point outside the file.
    pub fn sfx_sample(&self, sound: u8) -> Option<SfxSample> {
        let data = self.sound_data.as_ref()?;
        let sound = sound as usize;
        // Note: All those tables are big endian. Don't ask me why. *sigh*
        let be16 = |i: usize| data.get(i..i + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));

        // Use the sample rate from game data, see bug #2688.
        let table_rate = be16(self.sample_rates_ofs + (sound << 2))?;
        let sample_rate = table_rate.min(11025);
        let info = self.sfx_base_ofs as usize + (sound << 3);
        let data_ofs = ((be16(info)? as usize) << 4) + self.sfx_base_ofs as usize;
        let data_size = be16(info + 2)? as usize;
        let data_loop = be16(info + 6)? as usize;

        let end = data_ofs + data_size;
        if end > data.len() || sample_rate == 0 {
            log::warn!("sfx {sound}: {data_size} bytes at {data_ofs}, rate {sample_rate}, outside the file");
            return None;
        }
        let loop_start = if data_loop != 0 {
            // loopSta = dataSize - dataLoop; ScummVM does not guard a loop
            // longer than the sound, we play it from the start then.
            Some(data_ofs + data_size.saturating_sub(data_loop))
        } else {
            None
        };
        Some(SfxSample {
            pcm: Pcm { data: data.clone(), start: data_ofs, end, rate: sample_rate as u32, loop_start },
            table_rate,
        })
    }

    /// `playSound(sound, volume, channel)`: plays effect `sound` of the
    /// loaded section on channel 0 or 1 (any other value means 1, as in
    /// ScummVM), stopping what played there. `volume` is 0 to 127. The
    /// intro calls this directly; scripts go through `fn_start_fx`.
    pub fn play_sound(&mut self, sound: u16, volume: u16, channel: u8) {
        let id = if channel == 0 { SOUND_CH0 } else { SOUND_CH1 };
        self.stop_id(id);

        if self.sound_data.is_none() {
            log::warn!("Sound::playSound({sound:04X}, {volume:04X}) called with a section having been loaded");
            return;
        }

        // Checked before the number is cut to 8 bits, as in the original.
        if sound > self.sounds_total as u16 {
            log::debug!("Sound::playSound {sound} ignored, only {} sfx in file", self.sounds_total);
            return;
        }

        let volume = (volume & 0x7F) << 1;
        let sound = (sound & 0xFF) as u8;

        let Some(s) = self.sfx_sample(sound) else { return };
        self.play_id(id, s.pcm, volume, true);
    }

    /// `fnStartFx`: the script command. `sound` is an fx number (256 to
    /// 393; others are ignored), `channel` 0 or 1 (Logic passes `b & 1`).
    /// `screen` is `Logic::_scriptVariables[SCREEN]`, the current room;
    /// `system_flags` is `SkyEngine::_systemVars->systemFlags`, of which
    /// `SF_FX_OFF`, `SF_SBLASTER` and `SF_ROLAND` count.
    pub fn fn_start_fx(&mut self, sound: u32, channel: u8, screen: u32, system_flags: u32) {
        let channel = channel & 1;
        self.save_sounds[channel as usize] = 0xFFFF;
        if !(256..=MAX_FX_NUMBER).contains(&sound) || system_flags & SF_FX_OFF != 0 {
            return;
        }

        let screen = (screen & 0xff) as u8;
        let mut sound = sound;
        if sound == 278 && screen == 25 {
            // is this weld in room 25
            sound = 394;
        }

        sound &= !(1 << 8);

        let sfx = MUSIC_LIST[sound as usize];
        let room_list = &sfx.room_list;

        let mut i = 0;
        if room_list[i].room != 0xff {
            // if room list empty then do all rooms
            while room_list[i].room != screen {
                // check rooms
                i += 1;
                if room_list[i].room == 0xff {
                    return;
                }
            }
        }

        // get fx volume

        let mut volume = self.main_sfx_volume; // start with standard vol

        if system_flags & SF_SBLASTER != 0 {
            volume = room_list[i].adlib_volume;
        } else if system_flags & SF_ROLAND != 0 {
            volume = room_list[i].roland_volume;
        }
        volume = ((volume as u32 * self.main_sfx_volume as u32) >> 8) as u8;

        // Check the flags, the sound may come on after a delay.
        if sfx.flags & SFXF_START_DELAY != 0 {
            if let Some(q) = self.sfx_queue.iter_mut().find(|q| q.count == 0) {
                q.chan = channel;
                q.fx_no = sfx.sound_no;
                q.vol = volume;
                q.count = sfx.flags & 0x7F;
            }
            return; // ignore sound if it can't be queued
        }

        if sfx.flags & SFXF_SAVE != 0 {
            self.save_sounds[channel as usize] = sfx.sound_no as u16 | (volume as u16) << 8;
        }

        self.play_sound(sfx.sound_no as u16, volume as u16, channel);
    }

    /// `checkFxQueue`: once per game tick, counts down the delayed effects
    /// and starts those whose time has come.
    pub fn check_fx_queue(&mut self) {
        for cnt in 0..MAX_QUEUED_FX {
            if self.sfx_queue[cnt].count != 0 {
                self.sfx_queue[cnt].count -= 1;
                if self.sfx_queue[cnt].count == 0 {
                    let q = self.sfx_queue[cnt];
                    self.play_sound(q.fx_no as u16, q.vol as u16, q.chan);
                }
            }
        }
    }

    /// `restoreSfx`: after `save_sounds` has been read from a save game,
    /// queues those effects so they start on the next tick (when the player
    /// leaves the control panel).
    pub fn restore_sfx(&mut self) {
        // queue sfx, so they will be started when the player exits the control panel
        self.sfx_queue = [SfxQueue::default(); MAX_QUEUED_FX];
        let mut queue_slot = 0;
        for chan in 0..2 {
            let s = self.save_sounds[chan];
            if s != 0xFFFF {
                self.sfx_queue[queue_slot] =
                    SfxQueue { fx_no: s as u8, vol: (s >> 8) as u8, chan: chan as u8, count: 1 };
                queue_slot += 1;
            }
        }
    }

    /// Whether an effect still plays on `channel` (0 or 1). ScummVM has no
    /// such call; for tools and tests.
    pub fn fx_active(&self, channel: u8) -> bool {
        self.is_active(if channel == 0 { SOUND_CH0 } else { SOUND_CH1 })
    }

    /// The table entry for fx number `fx` (256 to 394): the sound number in
    /// the section's file, the flags, and the room list up to its end
    /// marker as (room, AdLib volume, Roland volume). For tools and tests.
    pub fn fx_entry(fx: u32) -> Option<(u8, u8, Vec<(u8, u8, u8)>)> {
        if !(256..=394).contains(&fx) {
            return None;
        }
        let sfx = MUSIC_LIST[(fx & !(1 << 8)) as usize];
        let mut rooms = Vec::new();
        for r in &sfx.room_list {
            rooms.push((r.room, r.adlib_volume, r.roland_volume));
            if r.room == 0xff {
                break;
            }
        }
        Some((sfx.sound_no, sfx.flags, rooms))
    }

    /// `fnStopFx`: stops both effect channels and forgets the effects to
    /// save.
    pub fn fn_stop_fx(&mut self) {
        self.stop_id(SOUND_CH0);
        self.stop_id(SOUND_CH1);
        self.save_sounds = [0xFFFF; 2];
    }

    /// `stopSpeech`.
    pub fn stop_speech(&mut self) {
        self.stop_id(SOUND_SPEECH);
    }

    /// The file number of the speech for `text_num`: the section in the top
    /// four bits picks a base from the table, the line number is added.
    /// Sections above 7 are not in the table and give `None` (ScummVM
    /// would read past it).
    pub fn speech_file(text_num: u16) -> Option<u16> {
        let base = *SPEECH_CONVERT_TABLE.get((text_num >> 12) as usize)?;
        Some(base + (text_num & 0xFFF) + 50000)
    }

    /// The speech for `text_num` as `startSpeech` plays it: the samples
    /// after the file's header, as many as its `s_tot_size` says, and the
    /// rate. `None` if there is no such file (not every line is spoken).
    pub fn speech_pcm(disk: &Disk, text_num: u16) -> Option<Pcm> {
        let file = Self::speech_file(text_num)?;
        let speech_data = disk.load(file);
        let Some(speech_data) = speech_data else {
            log::debug!(
                "File {file} (speechFile {} from section {}) wasn't found",
                text_num & 0xFFF,
                text_num >> 12
            );
            return None;
        };
        if speech_data.len() < HEADER_SIZE {
            return None;
        }

        // s_tot_size is the header's 16-bit total size, header included.
        let tot_size = u16::from_le_bytes([speech_data[12], speech_data[13]]) as usize;
        let speech_size = tot_size.saturating_sub(HEADER_SIZE);
        let end = (HEADER_SIZE + speech_size).min(speech_data.len());

        // Workaround for BASS bug #1461 - some voice-overs are played at
        // half speed in 0.0368 (the freeware CD version), in 0.0372 they sound
        // just fine.
        let rate = if disk.game_version == 368 && (text_num == 20905 || text_num == 20906) {
            22050
        } else {
            11025
        };

        Some(Pcm { data: speech_data.into(), start: HEADER_SIZE, end, rate, loop_start: None })
    }

    /// `startSpeech`: plays the spoken line for `text_num` and returns
    /// whether there was one to play. Nothing when `system_flags` lacks
    /// `SF_ALLOW_SPEECH`. Unlike ScummVM it also returns false without an
    /// audio device: speech would then count as finished at once, and
    /// Logic shows the text whenever no speech was found, so the line is
    /// read instead of skipped.
    pub fn start_speech(&mut self, disk: &Disk, text_num: u16, system_flags: u32) -> bool {
        if system_flags & SF_ALLOW_SPEECH == 0 {
            return false;
        }
        let Some(pcm) = Self::speech_pcm(disk, text_num) else { return false };
        self.stop_id(SOUND_SPEECH);
        if self.mixer.is_none() {
            return false;
        }
        self.play_id(SOUND_SPEECH, pcm, MAX_CHANNEL_VOLUME as u16, true);
        true
    }

    /// `speechFinished`: true when no speech is playing. Cheap; Logic asks
    /// every tick.
    pub fn speech_finished(&self) -> bool {
        !self.is_active(SOUND_SPEECH)
    }

    /// `fnPauseFx`: pauses both effect channels (game paused, or a script).
    pub fn fn_pause_fx(&mut self) {
        if !self.is_paused {
            self.is_paused = true;
            for id in [SOUND_CH0, SOUND_CH1] {
                if let Some(h) = &self.channels[id] {
                    h.set_paused(true);
                }
            }
        }
    }

    /// `fnUnPauseFx`.
    pub fn fn_un_pause_fx(&mut self) {
        if self.is_paused {
            self.is_paused = false;
            for id in [SOUND_CH0, SOUND_CH1] {
                if let Some(h) = &self.channels[id] {
                    h.set_paused(false);
                }
            }
        }
    }

    // The intro (intro.cpp) plays its voices and background sound on the
    // mixer itself; these are those calls, so it needs no mixer of its own.

    /// Intro `PLAYVOICE`: plays a whole voice file, header included, with
    /// the header filled with silence (127), at 11025 Hz, as speech.
    pub fn play_intro_voice(&mut self, mut file: Vec<u8>) {
        // HACK: Fill the header with silence. We should
        // probably use _skySound instead of calling playStream()
        // directly, but this will have to do for now.
        let n = HEADER_SIZE.min(file.len());
        file[..n].fill(127);
        // ScummVM plays it under SOUND_VOICE without stopping that id
        // first; the old voice plays on, but WAITVOICE asks after the new one.
        self.play_id(SOUND_VOICE, Pcm::once(file.into(), 11025), MAX_CHANNEL_VOLUME as u16, false);
    }

    /// Intro `WAITVOICE`: whether the last intro voice still plays.
    pub fn intro_voice_active(&self) -> bool {
        self.is_active(SOUND_VOICE)
    }

    /// Intro `PLAYBG` / `LOOPBG`: plays a background sound file loaded by
    /// `LOADBG`, skipping its first 256 bytes and leaving out the last 512
    /// (ScummVM plays `size - 768` bytes from offset 256), at 11025 Hz,
    /// once or looped whole.
    pub fn play_intro_bg(&mut self, file: Arc<[u8]>, looped: bool) {
        self.stop_id(SOUND_BG);
        if file.len() < 768 {
            return;
        }
        let end = file.len() - 512;
        let pcm = Pcm { data: file, start: 256, end, rate: 11025, loop_start: looped.then_some(256) };
        self.play_id(SOUND_BG, pcm, MAX_CHANNEL_VOLUME as u16, true);
    }

    /// Intro `STOPBG`, and the end of the intro.
    pub fn stop_intro_bg(&mut self) {
        self.stop_id(SOUND_BG);
    }
}

impl Drop for Sound {
    fn drop(&mut self) {
        self.stop_all();
    }
}

// The effect tables, generated from sound.cpp (names and comments kept).

static FX_NULL: Sfx = Sfx { sound_no: 0, flags: 0, room_list: [rl(200, 127, 127), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_LEVEL_3_PING: Sfx = Sfx { sound_no: 1, flags: 0, room_list: [rl(28, 63, 63), rl(29, 63, 63), rl(31, 63, 63), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_FACTORY_SOUND: Sfx = Sfx { sound_no: 1, flags: SFXF_SAVE, room_list: [rl(255, 30, 30), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_CROWBAR_PLASTER: Sfx = Sfx { sound_no: 1, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_MASONRY_FALL: Sfx = Sfx { sound_no: 1, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_PRISE_BRICK: Sfx = Sfx { sound_no: 2, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_ROPE_CREAK: Sfx = Sfx { sound_no: 2, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_PING: Sfx = Sfx { sound_no: 3, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_FORCE_FIRE_DOOR: Sfx = Sfx { sound_no: 3, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_BRICK_HIT_FOSTER: Sfx = Sfx { sound_no: 3, flags: 10 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_BRICK_HIT_PLANK: Sfx = Sfx { sound_no: 3, flags: 8 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_RM3_LIFT_MOVING: Sfx = Sfx { sound_no: 4, flags: SFXF_SAVE, room_list: [rl(3, 127, 127), rl(2, 127, 127), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_WELD: Sfx = Sfx { sound_no: 4, flags: 0, room_list: [rl(15, 127, 127), rl(7, 127, 127), rl(6, 60, 60), rl(12, 60, 60), rl(13, 60, 60), rl(255, 0, 0), RL0, RL0, RL0, RL0] };
static FX_WELD12: Sfx = Sfx { sound_no: 4, flags: 0, room_list: [rl(12, 127, 127), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_SPRAY_ON_SKIN: Sfx = Sfx { sound_no: 4, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_PLANK_VIBRATING: Sfx = Sfx { sound_no: 4, flags: 6 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_PRESS_BANG: Sfx = Sfx { sound_no: 5, flags: 0, room_list: [rl(0, 50, 100), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_SPANNER_CLUNK: Sfx = Sfx { sound_no: 5, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_BREAK_CRYSTALS: Sfx = Sfx { sound_no: 5, flags: 0, room_list: [rl(96, 127, 127), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_PRESS_HISS: Sfx = Sfx { sound_no: 6, flags: 0, room_list: [rl(0, 40, 40), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_OPEN_DOOR: Sfx = Sfx { sound_no: 6, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_OPEN_LAMB_DOOR: Sfx = Sfx { sound_no: 6, flags: 0, room_list: [rl(20, 127, 127), rl(21, 127, 127), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_SPLASH: Sfx = Sfx { sound_no: 6, flags: 22 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_DISINTEGRATE: Sfx = Sfx { sound_no: 7, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_BUZZER: Sfx = Sfx { sound_no: 7, flags: 4 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_LATHE: Sfx = Sfx { sound_no: 7, flags: SFXF_SAVE, room_list: [rl(4, 60, 60), rl(2, 20, 20), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_HIT_CROWBAR_BRICK: Sfx = Sfx { sound_no: 7, flags: 9 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_HELLO_HELGA: Sfx = Sfx { sound_no: 8, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_STATUE_ON_ARMOR: Sfx = Sfx { sound_no: 8, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_LIFT_ALARM: Sfx = Sfx { sound_no: 8, flags: SFXF_SAVE, room_list: [rl(2, 63, 63), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_DROP_CROWBAR: Sfx = Sfx { sound_no: 8, flags: 5 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_BYEE_HELGA: Sfx = Sfx { sound_no: 9, flags: 3 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_SHED_DOOR_CREAK: Sfx = Sfx { sound_no: 10, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_EXPLOSION: Sfx = Sfx { sound_no: 10, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_FIRE_CRACKLE_IN_PIT: Sfx = Sfx { sound_no: 9, flags: SFXF_SAVE, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_REMOVE_BAR_GRILL: Sfx = Sfx { sound_no: 10, flags: 7 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_GRILL_CREAK: Sfx = Sfx { sound_no: 10, flags: 43 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_STEAM1: Sfx = Sfx { sound_no: 11, flags: SFXF_SAVE, room_list: [rl(18, 20, 20), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_STEAM2: Sfx = Sfx { sound_no: 11, flags: SFXF_SAVE, room_list: [rl(18, 63, 63), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_STEAM3: Sfx = Sfx { sound_no: 11, flags: SFXF_SAVE, room_list: [rl(18, 127, 127), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_CROWBAR_WOODEN: Sfx = Sfx { sound_no: 11, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_HELMET_DOWN_3: Sfx = Sfx { sound_no: 11, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_GUARD_FALL: Sfx = Sfx { sound_no: 11, flags: 4, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
// Defined in sound.cpp but in no list: 274, the furnace room noise, is fx_null.
#[allow(dead_code)]
static FX_FURNACE: Sfx = Sfx { sound_no: 11, flags: 0, room_list: [rl(3, 90, 90), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_FALL_THRU_BOX: Sfx = Sfx { sound_no: 12, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_LAZER: Sfx = Sfx { sound_no: 12, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_SCANNER: Sfx = Sfx { sound_no: 12, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_HELMET_UP_3: Sfx = Sfx { sound_no: 12, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_LIQUID_BUBBLE: Sfx = Sfx { sound_no: 12, flags: SFXF_SAVE, room_list: [rl(80, 127, 127), rl(72, 127, 127), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_LIQUID_DRIP: Sfx = Sfx { sound_no: 13, flags: 6 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_GOO_DRIP: Sfx = Sfx { sound_no: 13, flags: 5 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_COMP_BLEEPS: Sfx = Sfx { sound_no: 13, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_USE_CROWBAR_GRILL: Sfx = Sfx { sound_no: 13, flags: 34 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_HELMET_GRIND: Sfx = Sfx { sound_no: 14, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_LIFT_MOVING: Sfx = Sfx { sound_no: 14, flags: SFXF_SAVE, room_list: [rl(7, 127, 127), rl(29, 127, 127), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_USE_SECATEURS: Sfx = Sfx { sound_no: 14, flags: 18 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_HIT_JOEY1: Sfx = Sfx { sound_no: 14, flags: 7 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_HIT_JOEY2: Sfx = Sfx { sound_no: 14, flags: 13 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_DANI_PHONE_RING: Sfx = Sfx { sound_no: 15, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_SC74_POD_DOWN: Sfx = Sfx { sound_no: 15, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_PHONE: Sfx = Sfx { sound_no: 15, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_25_WELD: Sfx = Sfx { sound_no: 15, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_LIFT_OPEN_7: Sfx = Sfx { sound_no: 15, flags: 0, room_list: [rl(7, 127, 127), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_LIFT_CLOSE_7: Sfx = Sfx { sound_no: 16, flags: 0, room_list: [rl(7, 127, 127), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_S2_HELMET: Sfx = Sfx { sound_no: 16, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_HISS_IN_NITROGEN: Sfx = Sfx { sound_no: 16, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_DOG_YAP_INDOORS: Sfx = Sfx { sound_no: 16, flags: 0, room_list: [rl(38, 127, 127), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_DOG_YAP_OUTDOORS: Sfx = Sfx { sound_no: 16, flags: 0, room_list: [rl(31, 127, 127), rl(30, 40, 40), rl(32, 40, 40), rl(33, 40, 40), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0] };
static FX_LOCKER_CREAK_OPEN: Sfx = Sfx { sound_no: 17, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_BIG_TENT_GURGLE: Sfx = Sfx { sound_no: 17, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_WIND_HOWL: Sfx = Sfx { sound_no: 17, flags: SFXF_SAVE, room_list: [rl(1, 127, 127), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_LIFT_OPEN_29: Sfx = Sfx { sound_no: 17, flags: 0, room_list: [rl(29, 127, 127), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_LIFT_ARRIVE_7: Sfx = Sfx { sound_no: 17, flags: 0, room_list: [rl(7, 63, 63), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_LIFT_CLOSE_29: Sfx = Sfx { sound_no: 18, flags: 0, room_list: [rl(29, 127, 127), rl(28, 127, 127), rl(255, 0, 0), RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_SHAFT_INDUSTRIAL_NOISE: Sfx = Sfx { sound_no: 18, flags: SFXF_SAVE, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_GALL_DROP: Sfx = Sfx { sound_no: 18, flags: 29 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_DOOR_SLAM_UNDER: Sfx = Sfx { sound_no: 19, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_REICHS_FISH: Sfx = Sfx { sound_no: 19, flags: SFXF_SAVE, room_list: [rl(255, 60, 60), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_JUDGES_GAVEL1: Sfx = Sfx { sound_no: 19, flags: 13 + SFXF_START_DELAY, room_list: [rl(255, 60, 60), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_JUDGES_GAVEL2: Sfx = Sfx { sound_no: 19, flags: 16 + SFXF_START_DELAY, room_list: [rl(255, 90, 90), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_JUDGES_GAVEL3: Sfx = Sfx { sound_no: 19, flags: 19 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_WIND_3: Sfx = Sfx { sound_no: 20, flags: SFXF_SAVE, room_list: [rl(255, 60, 60), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_FACT_SENSOR: Sfx = Sfx { sound_no: 20, flags: SFXF_SAVE, room_list: [rl(255, 60, 60), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_MEDI_STAB_GALL: Sfx = Sfx { sound_no: 20, flags: 17 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_COMPUTER_3: Sfx = Sfx { sound_no: 21, flags: SFXF_SAVE, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_TIMBER_CRACKING: Sfx = Sfx { sound_no: 21, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_ANCHOR_FALL: Sfx = Sfx { sound_no: 22, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_ELEVATOR_4: Sfx = Sfx { sound_no: 22, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_STAR_TREK_2: Sfx = Sfx { sound_no: 22, flags: SFXF_SAVE, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_LIFT_CLOSING: Sfx = Sfx { sound_no: 23, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_HEARTBEAT: Sfx = Sfx { sound_no: 23, flags: 11 + SFXF_START_DELAY, room_list: [rl(67, 60, 60), rl(68, 60, 60), rl(69, 60, 60), rl(77, 20, 20), rl(78, 50, 50), rl(79, 70, 70), rl(80, 127, 127), rl(81, 60, 60), rl(255, 0, 0), RL0] };
static FX_POS_KEY: Sfx = Sfx { sound_no: 25, flags: 2 + SFXF_START_DELAY, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_NEG_KEY: Sfx = Sfx { sound_no: 26, flags: 2 + SFXF_START_DELAY, room_list: [rl(255, 100, 100), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };
static FX_ORIFICE_SWALLOW_DRIP: Sfx = Sfx { sound_no: 28, flags: 0, room_list: [rl(255, 127, 127), RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0, RL0] };

/// `musicList`: fx 256 to 394, indexed by the fx number with bit 8 cleared.
static MUSIC_LIST: [&Sfx; 139] = [
    &FX_PRESS_BANG, // 256 banging of the press
    &FX_PRESS_HISS, // 257 hissing press
    &FX_WIND_HOWL, // 258 howling wind
    &FX_SPANNER_CLUNK, // 259 spanner in works
    &FX_REICHS_FISH, // 260 Reichs fish
    &FX_EXPLOSION, // 261 panel blows open
    &FX_WIND_3, // 262 single steam
    &FX_OPEN_DOOR, // 263 general open door
    &FX_OPEN_LAMB_DOOR, // 264 lamb door opens
    &FX_COMP_BLEEPS, // 265 scanner bleeps
    &FX_HELMET_DOWN_3, // 266
    &FX_HELMET_UP_3, // 267
    &FX_HELMET_GRIND, // 268
    &FX_LIFT_CLOSE_29, // 269 rm 29 lift closes
    &FX_LIFT_OPEN_29, // 270 rm 29 lift opens
    &FX_COMPUTER_3, // 271 rm 29 lift arrives
    &FX_LEVEL_3_PING, // 272 background noise in room 4
    &FX_LIFT_ALARM, // 273 loader alarm
    &FX_NULL, // 274 furnace room background noise
    &FX_RM3_LIFT_MOVING, // 275 lift moving in room 3
    &FX_LATHE, // 276 jobsworth lathe
    &FX_FACTORY_SOUND, // 277 factory background sound
    &FX_WELD, // 278 do some welding
    &FX_LIFT_CLOSE_7, // 279 rm 7 lift closes
    &FX_LIFT_OPEN_7, // 280 rm 7 lift opens
    &FX_LIFT_ARRIVE_7, // 281 rm 7 lift arrives
    &FX_LIFT_MOVING, // 282 lift moving
    &FX_SCANNER, // 283 scanner operating
    &FX_FORCE_FIRE_DOOR, // 284 Force fire door open
    &FX_NULL, // 285 General door creak
    &FX_PHONE, // 286 telephone
    &FX_LAZER, // 287 lazer
    &FX_LAZER, // 288 lazer
    &FX_ANCHOR_FALL, // 289 electric   ;not used on amiga
    &FX_WELD12, // 290 welding in room 12 (not joey)
    &FX_HELLO_HELGA, // 291 helga appears
    &FX_BYEE_HELGA, // 292 helga disapears
    &FX_NULL, // 293 smash through window               ;doesn't exist
    &FX_POS_KEY, // 294
    &FX_NEG_KEY, // 295
    &FX_S2_HELMET, // 296 ;helmet down section 2
    &FX_S2_HELMET, // 297 ;  "      up    "    "
    &FX_LIFT_ARRIVE_7, // 298 ;security door room 7
    &FX_NULL, // 299
    &FX_ROPE_CREAK, // 300
    &FX_CROWBAR_WOODEN, // 301
    &FX_FALL_THRU_BOX, // 302
    &FX_USE_CROWBAR_GRILL, // 303
    &FX_USE_SECATEURS, // 304
    &FX_GRILL_CREAK, // 305
    &FX_TIMBER_CRACKING, // 306
    &FX_MASONRY_FALL, // 307
    &FX_MASONRY_FALL, // 308
    &FX_CROWBAR_PLASTER, // 309
    &FX_PRISE_BRICK, // 310
    &FX_BRICK_HIT_FOSTER, // 311
    &FX_SPRAY_ON_SKIN, // 312
    &FX_HIT_CROWBAR_BRICK, // 313
    &FX_DROP_CROWBAR, // 314
    &FX_FIRE_CRACKLE_IN_PIT, // 315
    &FX_REMOVE_BAR_GRILL, // 316
    &FX_LIQUID_BUBBLE, // 317
    &FX_LIQUID_DRIP, // 318
    &FX_GUARD_FALL, // 319
    &FX_SC74_POD_DOWN, // 320
    &FX_HISS_IN_NITROGEN, // 321
    &FX_NULL, // 322
    &FX_HIT_JOEY1, // 323
    &FX_HIT_JOEY2, // 324
    &FX_MEDI_STAB_GALL, // 325
    &FX_GALL_DROP, // 326
    &FX_NULL, // 327
    &FX_NULL, // 328
    &FX_NULL, // 329
    &FX_BIG_TENT_GURGLE, // 330
    &FX_NULL, // 331
    &FX_ORIFICE_SWALLOW_DRIP, // 332
    &FX_BRICK_HIT_PLANK, // 333
    &FX_GOO_DRIP, // 334
    &FX_PLANK_VIBRATING, // 335
    &FX_SPLASH, // 336
    &FX_BUZZER, // 337
    &FX_SHED_DOOR_CREAK, // 338
    &FX_DOG_YAP_OUTDOORS, // 339
    &FX_DANI_PHONE_RING, // 340
    &FX_LOCKER_CREAK_OPEN, // 341
    &FX_JUDGES_GAVEL1, // 342
    &FX_DOG_YAP_INDOORS, // 343
    &FX_BRICK_HIT_PLANK, // 344
    &FX_BRICK_HIT_PLANK, // 345
    &FX_SHAFT_INDUSTRIAL_NOISE, // 346
    &FX_JUDGES_GAVEL2, // 347
    &FX_JUDGES_GAVEL3, // 348
    &FX_ELEVATOR_4, // 349
    &FX_LIFT_CLOSING, // 350
    &FX_NULL, // 351
    &FX_NULL, // 352
    &FX_SC74_POD_DOWN, // 353
    &FX_NULL, // 354
    &FX_NULL, // 355
    &FX_HEARTBEAT, // 356
    &FX_STAR_TREK_2, // 357
    &FX_NULL, // 358
    &FX_NULL, // 359
    &FX_NULL, // 350
    &FX_NULL, // 361
    &FX_NULL, // 362
    &FX_NULL, // 363
    &FX_NULL, // 364
    &FX_NULL, // 365
    &FX_BREAK_CRYSTALS, // 366
    &FX_DISINTEGRATE, // 367
    &FX_STATUE_ON_ARMOR, // 368
    &FX_NULL, // 369
    &FX_NULL, // 360
    &FX_PING, // 371
    &FX_NULL, // 372
    &FX_DOOR_SLAM_UNDER, // 373
    &FX_NULL, // 374
    &FX_NULL, // 375
    &FX_NULL, // 376
    &FX_NULL, // 377
    &FX_NULL, // 378
    &FX_NULL, // 379
    &FX_STEAM1, // 380
    &FX_STEAM2, // 381
    &FX_STEAM2, // 382
    &FX_STEAM3, // 383
    &FX_NULL, // 384
    &FX_NULL, // 385
    &FX_FACT_SENSOR, // 386            Sensor in Potts' room
    &FX_NULL, // 387
    &FX_NULL, // 388
    &FX_NULL, // 389
    &FX_NULL, // 390
    &FX_NULL, // 391
    &FX_NULL, // 392
    &FX_NULL, // 393
    &FX_25_WELD, // 394            my anchor weld bodge
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The CD version's data; tests that need it pass when it is absent.
    const DATA: &str = "/home/david/store/code/asciiverse/games/beneath-a-typographic-sky/reference/bass-cd-1.2";

    fn disk() -> Option<Disk> {
        let p = Path::new(DATA);
        if !p.join("sky.dsk").exists() {
            eprintln!("skipping: no game data at {DATA}");
            return None;
        }
        Some(Disk::open(p).expect("sky.dnr/sky.dsk"))
    }

    /// The sections with a sound file: 0 for the intro, 1 to 6 as Logic
    /// loads them (its section plus one).
    const SECTIONS: std::ops::RangeInclusive<u8> = 0..=6;

    #[test]
    fn tables_are_complete() {
        assert_eq!(MUSIC_LIST.len(), 394 - 256 + 1);
        for fx in 256..=394 {
            let (_, _, rooms) = Sound::fx_entry(fx).unwrap();
            assert_eq!(rooms.last().unwrap().0, 0xff, "fx {fx} room list has no end");
        }
        assert_eq!(Sound::speech_file(0x1001), Some(50601));
        assert_eq!(Sound::speech_file(0x6000), Some(50000 + 600 + 500 + 1330 + 950 + 1150 + 550));
        assert_eq!(Sound::speech_file(0x8000), None);
    }

    #[test]
    fn every_sound_of_every_section_decodes() {
        let Some(disk) = disk() else { return };
        let mut s = Sound::new(&Audio::silent(), MAX_CHANNEL_VOLUME);
        let mut max_total = 0;
        for section in SECTIONS {
            s.load_section(&disk, section, disk.game_version, 0).unwrap();
            assert!(s.sounds_total >= 1, "section {section}");
            max_total = max_total.max(s.sounds_total);
            for n in 0..=s.sounds_total {
                let smp = s.sfx_sample(n).unwrap_or_else(|| panic!("section {section} sound {n}"));
                assert!(smp.pcm.len() > 0 && smp.pcm.rate > 0 && smp.pcm.rate <= 11025);
                if let Some(l) = smp.pcm.loop_start {
                    assert!(l >= smp.pcm.start && l < smp.pcm.end, "section {section} sound {n} loop");
                }
            }
        }
        // one past the last section has no file
        assert!(s.load_section(&disk, 7, disk.game_version, 0).is_err());

        // every effect in the table names a sound some section has
        for fx in 256..=394 {
            let (no, _, _) = Sound::fx_entry(fx).unwrap();
            assert!(no <= max_total, "fx {fx} wants sound {no}");
        }
    }

    #[test]
    fn effects_resolve_in_their_rooms() {
        let Some(disk) = disk() else { return };
        let mut s = Sound::new(&Audio::silent(), MAX_CHANNEL_VOLUME);
        s.load_section(&disk, 1, disk.game_version, 0).unwrap();
        let vol30 = (30 * 255 >> 8) << 8;

        // 277 factory background: every room, saved, AdLib volume 30
        s.fn_start_fx(277, 1, 3, SF_SBLASTER);
        assert_eq!(s.save_sounds, [0xFFFF, 1 | vol30]);
        // Roland volume when the flags say Roland
        s.fn_start_fx(277, 0, 3, SF_ROLAND);
        assert_eq!(s.save_sounds[0], 1 | vol30);
        // nothing when effects are off, and the saved one is forgotten
        s.fn_start_fx(277, 0, 3, SF_SBLASTER | SF_FX_OFF);
        assert_eq!(s.save_sounds[0], 0xFFFF);

        // 275 lift moving in room 3: only rooms 3 and 2
        s.fn_start_fx(275, 0, 3, SF_SBLASTER);
        assert_eq!(s.save_sounds[0] & 0xFF, 4);
        s.fn_start_fx(275, 0, 5, SF_SBLASTER);
        assert_eq!(s.save_sounds[0], 0xFFFF);

        // weld (278) is not listed for room 25; its stand-in 394 plays in
        // every room
        let (_, _, rooms394) = Sound::fx_entry(394).unwrap();
        assert_eq!(rooms394[0].0, 0xff);
        let (_, _, rooms278) = Sound::fx_entry(278).unwrap();
        assert!(!rooms278.iter().any(|r| r.0 == 25));

        // out of range numbers are ignored, 394 included when asked for directly
        s.fn_start_fx(255, 0, 3, SF_SBLASTER);
        s.fn_start_fx(394, 0, 25, SF_SBLASTER);
        assert!(s.sfx_queue().iter().all(|q| q.count == 0));
    }

    #[test]
    fn delayed_effects_wait_their_ticks() {
        let Some(disk) = disk() else { return };
        let mut s = Sound::new(&Audio::silent(), MAX_CHANNEL_VOLUME);
        s.load_section(&disk, 1, disk.game_version, 0).unwrap();
        // fx_brick_hit_foster: sound 3 after 10 ticks, every room
        let fx = (256..=394)
            .find(|&f| {
                let (no, flags, rooms) = Sound::fx_entry(f).unwrap();
                no == 3 && flags == 10 + SFXF_START_DELAY && rooms[0].0 == 0xff
            })
            .expect("brick hit foster");
        s.fn_start_fx(fx, 1, 7, SF_SBLASTER);
        assert_eq!(s.sfx_queue()[0], SfxQueue { count: 10, fx_no: 3, chan: 1, vol: 126 });
        for left in (0..10).rev() {
            s.check_fx_queue();
            assert_eq!(s.sfx_queue()[0].count, left);
        }
        // a full queue drops the fifth
        for _ in 0..5 {
            s.fn_start_fx(fx, 0, 7, SF_SBLASTER);
        }
        assert!(s.sfx_queue().iter().all(|q| q.count == 10));
        // a new section clears the queue, unless the game was just restored
        s.load_section(&disk, 1, disk.game_version, SF_GAME_RESTORED).unwrap();
        assert!(s.sfx_queue().iter().all(|q| q.count == 10));
        s.load_section(&disk, 1, disk.game_version, 0).unwrap();
        assert!(s.sfx_queue().iter().all(|q| q.count == 0));
    }

    #[test]
    fn restore_queues_saved_sounds() {
        let mut s = Sound::new(&Audio::silent(), MAX_CHANNEL_VOLUME);
        s.save_sounds = [0xFFFF, 0x1D01];
        s.restore_sfx();
        assert_eq!(s.sfx_queue()[0], SfxQueue { count: 1, fx_no: 1, chan: 1, vol: 0x1D });
        assert_eq!(s.sfx_queue()[1].count, 0);
        s.save_sounds = [0x7E04, 0x1D01];
        s.restore_sfx();
        assert_eq!(s.sfx_queue()[0], SfxQueue { count: 1, fx_no: 4, chan: 0, vol: 0x7E });
        assert_eq!(s.sfx_queue()[1], SfxQueue { count: 1, fx_no: 1, chan: 1, vol: 0x1D });
        s.fn_stop_fx();
        assert_eq!(s.save_sounds, [0xFFFF; 2]);
    }

    #[test]
    fn speech_decodes() {
        let Some(disk) = disk() else { return };
        // the first spoken line near the start of each section
        let mut found = 0;
        for section in 0..=6u16 {
            for line in 0..40 {
                let text = section << 12 | line;
                let Some(pcm) = Sound::speech_pcm(&disk, text) else { continue };
                let secs = pcm.duration().as_secs_f32();
                assert_eq!(pcm.rate, 11025);
                assert!((0.2..30.0).contains(&secs), "text {text:#x}: {secs} s");
                let peak = (pcm.start..pcm.end).map(|i| pcm.sample(i).abs()).fold(0.0, f32::max);
                assert!(peak > 0.1, "text {text:#x}: silent");
                found += 1;
                break;
            }
        }
        assert!(found >= 6, "only {found} sections have speech");

        // the 0.0368 half speed workaround does not apply to 0.0372
        assert_eq!(disk.game_version, 372);
        assert_eq!(Sound::speech_pcm(&disk, 20905).unwrap().rate, 11025);

        // no device: no speech started, so Logic shows the text
        let mut s = Sound::new(&Audio::silent(), MAX_CHANNEL_VOLUME);
        assert!(!s.start_speech(&disk, 0x1001, SF_ALLOW_SPEECH));
        assert!(!s.start_speech(&disk, 0x1001, 0));
        assert!(s.speech_finished());
    }

    #[test]
    fn every_speech_file_fits_its_header() {
        let Some(disk) = disk() else { return };
        let mut n = 0;
        for file in 50000..=Sound::speech_file(0x6FFF).unwrap() {
            let Some(d) = disk.load(file) else { continue };
            let tot = u16::from_le_bytes([d[12], d[13]]) as usize;
            assert_eq!(tot, d.len(), "speech file {file}");
            n += 1;
        }
        assert!(n > 3000, "{n} speech files");
    }
}

