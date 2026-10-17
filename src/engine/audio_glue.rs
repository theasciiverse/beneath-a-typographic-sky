//! Where the engine calls sound and music. Kept in one place so the audio
//! back end can be swapped in without touching the logic. Until it is
//! connected, speech counts as finished at once and nothing plays.

use super::sky::Sky;

#[derive(Default)]
pub struct AudioGlue {
    pub section: u8,
    pub music: u16,
}

impl Sky {
    pub(crate) fn audio_init(&mut self) {}

    pub(crate) fn audio_load_section(&mut self, section: u8) {
        self.audio.section = section;
    }

    pub(crate) fn audio_start_music(&mut self, n: u16) {
        self.audio.music = n;
    }

    pub(crate) fn audio_stop_music(&mut self) {
        self.audio.music = 0;
    }

    pub(crate) fn audio_start_fx(&mut self, _sound: u32, _channel: u8) {}

    pub(crate) fn audio_stop_fx(&mut self) {}

    pub(crate) fn audio_pause_fx(&mut self) {}

    pub(crate) fn audio_unpause_fx(&mut self) {}

    pub(crate) fn audio_check_fx_queue(&mut self) {}

    pub(crate) fn audio_start_speech(&mut self, _text: u16) -> bool {
        false
    }

    pub(crate) fn audio_speech_finished(&self) -> bool {
        true
    }

    pub(crate) fn audio_stop_speech(&mut self) {}

    pub(crate) fn audio_save_sounds(&self) -> [u16; 2] {
        [0, 0]
    }

    pub(crate) fn audio_restore_sfx(&mut self, _saved: [u16; 2]) {}
}
