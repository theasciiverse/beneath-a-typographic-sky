//! Where the engine calls sound and music, in one place: the sound effects
//! and speech (sound.rs) and the AdLib music (music/) on one shared output
//! (crate::audio). With no audio device, or a host that wants none,
//! nothing plays and speech counts as missing, so the text shows instead.

use super::music::MusicHandle;
use super::sky::Sky;
use super::sound::{Sound, MAX_CHANNEL_VOLUME};
use crate::audio::Audio;

pub struct AudioGlue {
    pub audio: Audio,
    pub sound: Sound,
    pub music: MusicHandle,
}

impl Default for AudioGlue {
    fn default() -> Self {
        let audio = Audio::silent();
        let sound = Sound::new(&audio, MAX_CHANNEL_VOLUME);
        AudioGlue { audio, sound, music: MusicHandle::new() }
    }
}

impl Sky {
    pub(crate) fn audio_init(&mut self) {
        if !self.host.wants_audio() {
            return;
        }
        let audio = Audio::new();
        let sound = Sound::new(&audio, MAX_CHANNEL_VOLUME);
        let music = MusicHandle::new();
        // ScummVM's default music volume: 192 of 256, which is 96 of 127
        music.set_volume(96);
        audio.add_source(music.source());
        log::debug!("audio device: {}", audio.available());
        self.audio = AudioGlue { audio, sound, music };
    }

    pub(crate) fn audio_load_section(&mut self, section: u8) {
        self.audio.music.load_section(&self.disk, section);
        let flags = self.sys.system_flags;
        if let Err(e) = self.audio.sound.load_section(&self.disk, section, self.sys.game_version, flags) {
            log::warn!("sound section {section}: {e:?}");
        }
    }

    pub(crate) fn audio_start_music(&mut self, n: u16) {
        log::debug!("music {n}");
        self.audio.music.start_music(n);
    }

    pub(crate) fn audio_stop_music(&mut self) {
        self.audio.music.stop_music();
    }

    pub(crate) fn audio_start_fx(&mut self, sound: u32, channel: u8) {
        let screen = self.vars[super::defs::SCREEN as usize];
        let flags = self.sys.system_flags;
        self.audio.sound.fn_start_fx(sound, channel, screen, flags);
    }

    pub(crate) fn audio_stop_fx(&mut self) {
        self.audio.sound.fn_stop_fx();
    }

    pub(crate) fn audio_pause_fx(&mut self) {
        self.audio.sound.fn_pause_fx();
    }

    pub(crate) fn audio_unpause_fx(&mut self) {
        self.audio.sound.fn_un_pause_fx();
    }

    pub(crate) fn audio_check_fx_queue(&mut self) {
        self.audio.sound.check_fx_queue();
    }

    pub(crate) fn audio_start_speech(&mut self, text: u16) -> bool {
        let flags = self.sys.system_flags;
        self.audio.sound.start_speech(&self.disk, text, flags)
    }

    pub(crate) fn audio_speech_finished(&self) -> bool {
        self.audio.sound.speech_finished()
    }

    pub(crate) fn audio_stop_speech(&mut self) {
        self.audio.sound.stop_speech();
    }

    pub(crate) fn audio_save_sounds(&self) -> [u16; 2] {
        self.audio.sound.save_sounds
    }

    pub(crate) fn audio_restore_sfx(&mut self, saved: [u16; 2]) {
        self.audio.sound.save_sounds = saved;
        self.audio.sound.restore_sfx();
    }
}
