//! Music: the game's AdLib driver on an emulated OPL2. Port of ScummVM's
//! engines/sky/music (MusicBase, AdLibMusic, AdLibChannel; the General MIDI
//! and MT-32 drivers are not ported).
//!
//! The driver lives behind an `Arc<Mutex<..>>`, as ScummVM's runs under
//! its `_mutex`: the game thread calls `MusicHandle`'s methods and the
//! audio thread pulls samples from `MusicSource`, which runs the driver's
//! 50 Hz timer at its place between the samples, as ScummVM's emulated
//! OPL does.
//!
//! The source is mono at 49716 Hz, the OPL2's own rate (its 14.318 MHz
//! clock divided by 288), so the chip produces exactly one sample per
//! internal cycle and never resamples; the mixer converts to the device
//! rate once. ScummVM instead asks Nuked-OPL3 for the mixer's rate.

// the engine API; not all of it is called yet
#![allow(dead_code)]

mod adlibchannel;
mod adlibmusic;
mod musicbase;
mod opl;

use std::num::NonZero;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::disk::Disk;
pub use adlibmusic::AdLibMusic;

/// The OPL2's native sample rate, 14318180 / 288.
pub const SAMPLE_RATE: u32 = 49716;

/// Samples rendered per lock of the driver.
const CHUNK: usize = 512;

/// The game's handle on the music. Cheap to clone; every clone and every
/// `MusicSource` made from it drive the same music.
#[derive(Clone)]
pub struct MusicHandle {
    driver: Arc<Mutex<AdLibMusic>>,
}

impl Default for MusicHandle {
    fn default() -> Self {
        MusicHandle::new()
    }
}

impl MusicHandle {
    pub fn new() -> MusicHandle {
        MusicHandle::with_rate(SAMPLE_RATE)
    }

    /// A driver rendering at `rate` samples a second instead of the OPL2's own.
    pub fn with_rate(rate: u32) -> MusicHandle {
        MusicHandle { driver: Arc::new(Mutex::new(AdLibMusic::new(rate))) }
    }

    fn lock(&self) -> MutexGuard<'_, AdLibMusic> {
        // a panic on the audio thread must not silence the game for good
        self.driver.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `loadSection`: stops the music and loads `section`'s driver and
    /// songs (sections 0 to 6 in the full game).
    pub fn load_section(&self, disk: &Disk, section: u8) {
        let file = self.lock().section_file(section);
        let data = disk.load(file);
        self.lock().load_section(section, data, disk.game_version);
    }

    /// `startMusic`: song `param & 0xF` of the loaded section starts at
    /// the next timer tick; 0 stops the music.
    pub fn start_music(&self, param: u16) {
        self.lock().start_music(param);
    }

    /// `stopMusic`; see `AdLibMusic::stop_music` for its quirk.
    pub fn stop_music(&self) {
        self.lock().stop_music();
    }

    pub fn music_is_playing(&self) -> bool {
        self.lock().music_is_playing()
    }

    /// `setVolume`, 0 to 127 as the game uses it.
    pub fn set_volume(&self, volume: u16) {
        self.lock().set_volume(volume);
    }

    pub fn give_volume(&self) -> u8 {
        self.lock().give_volume()
    }

    pub fn give_current_music(&self) -> u8 {
        self.lock().give_current_music()
    }

    /// Songs in the loaded section.
    pub fn song_count(&self) -> u8 {
        self.lock().song_count()
    }

    /// Renders the next `out.len()` samples directly, for tools and tests.
    /// Do not use it while a `MusicSource` of the same handle plays.
    pub fn render(&self, out: &mut [i16]) {
        self.lock().read_buffer(out);
    }

    pub fn sample_rate(&self) -> u32 {
        self.lock().opl.rate()
    }

    /// The endless audio source to add to the mixer. Make only one per
    /// handle: each pulls the same driver forward.
    pub fn source(&self) -> MusicSource {
        let rate = self.sample_rate();
        MusicSource { driver: self.driver.clone(), rate, buf: vec![0; CHUNK], pos: CHUNK }
    }
}

/// Endless mono rodio source playing the music; silent when nothing plays.
pub struct MusicSource {
    driver: Arc<Mutex<AdLibMusic>>,
    rate: u32,
    buf: Vec<i16>,
    pos: usize,
}

impl Iterator for MusicSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.pos == self.buf.len() {
            let mut d = self.driver.lock().unwrap_or_else(|e| e.into_inner());
            d.read_buffer(&mut self.buf);
            self.pos = 0;
        }
        let s = self.buf[self.pos];
        self.pos += 1;
        Some(s as f32 / 32768.0)
    }
}

impl rodio::Source for MusicSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> rodio::ChannelCount {
        NonZero::new(1).unwrap()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        NonZero::new(self.rate).unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn disk() -> Option<Disk> {
        let dir = std::env::var_os("TYPOSKY_DATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("reference/bass-cd-1.2"));
        match Disk::open(&dir) {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("skipping, no game data at {}: {e}", dir.display());
                None
            }
        }
    }

    struct Stats {
        rms: f64,
        peak: i16,
        clipped: usize,
    }

    fn stats(s: &[i16]) -> Stats {
        let sq: f64 = s.iter().map(|&x| (x as f64) * (x as f64)).sum();
        Stats {
            rms: (sq / s.len() as f64).sqrt(),
            peak: s.iter().map(|x| x.saturating_abs()).max().unwrap_or(0),
            clipped: s.iter().filter(|&&x| x == i16::MAX || x == i16::MIN).count(),
        }
    }

    #[test]
    fn every_section_loads_and_every_song_sounds() {
        let Some(disk) = disk() else { return };
        let music = MusicHandle::new();
        for section in 0..7u8 {
            music.load_section(&disk, section);
            let songs = music.song_count();
            assert!(songs > 0, "section {section} has no songs");
            for song in 1..=songs {
                music.load_section(&disk, section);
                music.start_music(song as u16);
                let mut buf = vec![0i16; SAMPLE_RATE as usize * 4];
                music.render(&mut buf);
                assert_eq!(music.give_current_music(), song);
                let st = stats(&buf);
                assert!(st.rms > 50.0, "section {section} song {song} is silent (rms {})", st.rms);
                assert!(
                    st.clipped < buf.len() / 1000,
                    "section {section} song {song} clips {} samples (peak {})",
                    st.clipped,
                    st.peak
                );
            }
        }
    }

    /// Plays `section`'s `song` for up to `seconds`; the samples, and
    /// whether it was still playing at the end.
    fn play(disk: &Disk, section: u8, song: u16, seconds: u32) -> (Vec<i16>, bool) {
        let music = MusicHandle::new();
        music.load_section(disk, section);
        music.start_music(song);
        let mut out = Vec::new();
        let mut chunk = vec![0i16; SAMPLE_RATE as usize];
        for _ in 0..seconds {
            music.render(&mut chunk);
            out.extend_from_slice(&chunk);
            if !music.music_is_playing() {
                return (out, false);
            }
        }
        (out, true)
    }

    /// ScummVM's MusicBase knows which of the Music Enhancement Project's
    /// recordings are the same tune in another section and which must not
    /// loop; the AdLib data should agree.
    #[test]
    fn songs_agree_with_scummvm_track_tables() {
        let Some(disk) = disk() else { return };
        // "handle duplicates" in MusicBase::startMusic
        let (one_one, _) = play(&disk, 1, 1, 20);
        assert_eq!(one_one, play(&disk, 5, 1, 20).0);
        let (one_four, _) = play(&disk, 1, 4, 20);
        // section 2's copy is the same for its first 9.5 s, then differs
        // a little, as its own data file has it (not looked into further)
        let nine = 9 * SAMPLE_RATE as usize;
        assert_eq!(one_four[..nine], play(&disk, 2, 4, 9).0[..nine]);
        assert_eq!(one_four, play(&disk, 5, 4, 20).0);
        // "not all tracks should loop" in MusicBase::loadNewMusic
        for (section, song) in [(0, 1), (1, 1), (1, 4), (2, 1), (2, 4), (4, 2), (4, 3), (4, 5), (4, 6), (4, 11), (5, 1), (5, 3), (5, 4)] {
            assert!(!play(&disk, section, song, 140).1, "section {section} song {song} should end");
        }
        for (section, song) in [(1, 2), (3, 1), (4, 4), (6, 1)] {
            assert!(play(&disk, section, song, 140).1, "section {section} song {song} should loop");
        }
    }

    #[test]
    fn stop_and_volume() {
        let Some(disk) = disk() else { return };
        let music = MusicHandle::new();
        music.load_section(&disk, 1);
        music.start_music(2);
        let mut buf = vec![0i16; SAMPLE_RATE as usize];
        music.render(&mut buf);
        assert!(music.music_is_playing());
        let loud = stats(&buf).rms;

        music.set_volume(30);
        assert_eq!(music.give_volume(), 30);
        music.render(&mut buf);
        let quiet = stats(&buf).rms;
        assert!(quiet < loud, "volume 30 ({quiet}) not quieter than 127 ({loud})");

        music.start_music(0);
        music.render(&mut buf);
        assert_eq!(music.give_current_music(), 0);
        assert!(!music.music_is_playing());
        // let the release tails die away
        music.render(&mut buf);
        music.render(&mut buf);
        assert!(stats(&buf[buf.len() / 2..]).rms < 5.0);
    }

    #[test]
    fn source_is_endless_and_matches_render() {
        let Some(disk) = disk() else { return };
        let a = MusicHandle::new();
        let b = MusicHandle::new();
        for m in [&a, &b] {
            m.load_section(&disk, 0);
            m.start_music(1);
        }
        let mut direct = vec![0i16; 20_000];
        a.render(&mut direct);
        let from_source: Vec<f32> = b.source().take(20_000).collect();
        for (x, y) in direct.iter().zip(&from_source) {
            assert_eq!(*x as f32 / 32768.0, *y);
        }
    }
}
