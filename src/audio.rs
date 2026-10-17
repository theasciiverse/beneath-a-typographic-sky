//! The shared audio output: one rodio mixer on the default device, which the
//! sound effects and speech (`engine::sound`) and the music player all add
//! their sources to.
//!
//! It stands in for ScummVM's `Audio::Mixer`. ScummVM addresses a playing
//! sound by its handle or its id; here every sound started through
//! [`Audio::play`] gets a [`Handle`] that can stop it, pause it, change its
//! volume and tell whether it is still playing, all without blocking, so the
//! game can ask every tick.
//!
//! Without an audio device (no sound card, CI, tests) nothing is played:
//! [`Audio::play`] returns `None`, [`Audio::add_source`] drops the source.

// Parts are for the engine loop and the music player, not written yet.
#![allow(dead_code)]

use std::num::NonZero;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rodio::mixer::Mixer;
use rodio::{MixerDeviceSink, Source};

pub struct Audio {
    sink: Option<MixerDeviceSink>,
}

impl Audio {
    /// Opens the default output device. Failing that, a silent `Audio`.
    pub fn new() -> Audio {
        let sink = match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(mut s) => {
                s.log_on_drop(false);
                Some(s)
            }
            Err(e) => {
                log::warn!("no audio: {e}");
                None
            }
        };
        Audio { sink }
    }

    /// An `Audio` that never opens a device: everything is silent.
    pub fn silent() -> Audio {
        Audio { sink: None }
    }

    /// Whether there is a device to play on.
    pub fn available(&self) -> bool {
        self.sink.is_some()
    }

    /// The mixer, for a module that keeps its own copy (it is cheap to
    /// clone). `None` without a device.
    pub fn mixer(&self) -> Option<Mixer> {
        self.sink.as_ref().map(|s| s.mixer().clone())
    }

    /// Adds a source to the mix as it is, for sources that manage
    /// themselves, like an endless music player. Dropped without a device.
    pub fn add_source<S: Source + Send + 'static>(&self, source: S) {
        if let Some(s) = &self.sink {
            s.mixer().add(source);
        }
    }

    /// Plays 8-bit unsigned mono PCM (see [`Pcm`]) and returns its handle;
    /// `None` without a device.
    pub fn play(&self, pcm: Pcm, gain: f32) -> Option<Handle> {
        let mixer = self.sink.as_ref()?.mixer();
        Some(play_on(mixer, pcm, gain))
    }
}

/// Plays `pcm` on `mixer` and returns its handle. For modules that keep a
/// clone of the mixer instead of the whole `Audio`.
pub fn play_on(mixer: &Mixer, pcm: Pcm, gain: f32) -> Handle {
    let handle = Handle::new(gain);
    mixer.add(PcmSource { pcm, handle: handle.clone() });
    handle
}

/// A sound in the original's format: 8-bit unsigned mono samples, as
/// ScummVM's `makeRawStream(..., FLAG_UNSIGNED)` plays them, optionally
/// looping.
#[derive(Clone)]
pub struct Pcm {
    /// The buffer the samples are in; shared, so a section's whole sound
    /// file can back all its effects without copying.
    pub data: Arc<[u8]>,
    /// First sample in `data`.
    pub start: usize,
    /// One past the last sample in `data`.
    pub end: usize,
    pub rate: u32,
    /// Where to jump back to (an index into `data`, at or after `start`)
    /// on reaching `end`. `None` plays once. As ScummVM's
    /// `makeLoopingAudioStream(stream, loopStart, loopEnd, 0)`: the whole
    /// sound plays first, then `loop_start..end` repeats forever.
    pub loop_start: Option<usize>,
}

impl Pcm {
    /// A whole buffer, played once.
    pub fn once(data: Arc<[u8]>, rate: u32) -> Pcm {
        let end = data.len();
        Pcm { data, start: 0, end, rate, loop_start: None }
    }

    /// Number of samples played before any looping.
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.len() as f64 / self.rate.max(1) as f64)
    }

    /// Sample `i` (an index into `data`) as rodio wants it, -1 to 1:
    /// ScummVM turns `b` into `(b << 8) ^ 0x8000` as a signed 16-bit sample,
    /// which is `(b - 128) / 128` of full scale.
    pub fn sample(&self, i: usize) -> f32 {
        (self.data[i] as f32 - 128.0) / 128.0
    }
}

struct State {
    stopped: AtomicBool,
    paused: AtomicBool,
    finished: AtomicBool,
    gain: AtomicU32,
}

/// Control over one playing sound, shared with its source in the mixer.
/// Cheap to clone; all methods are lock free.
#[derive(Clone)]
pub struct Handle(Arc<State>);

impl Handle {
    fn new(gain: f32) -> Handle {
        Handle(Arc::new(State {
            stopped: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            gain: AtomicU32::new(gain.to_bits()),
        }))
    }

    /// Stops the sound. It counts as finished at once, as after ScummVM's
    /// `stopID`, even if the mixer has a few samples of it left to drop.
    pub fn stop(&self) {
        self.0.stopped.store(true, Ordering::Relaxed);
        self.0.finished.store(true, Ordering::Relaxed);
    }

    /// While paused the sound keeps its place and plays silence.
    pub fn set_paused(&self, paused: bool) {
        self.0.paused.store(paused, Ordering::Relaxed);
    }

    pub fn set_gain(&self, gain: f32) {
        self.0.gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    /// ScummVM's `isSoundHandleActive`: false once the sound has played to
    /// its end or been stopped. A paused sound is still active.
    pub fn is_active(&self) -> bool {
        !self.0.finished.load(Ordering::Relaxed)
    }
}

/// The source in the mixer behind a [`Handle`].
struct PcmSource {
    pcm: Pcm,
    handle: Handle,
}

impl Iterator for PcmSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let st = &self.handle.0;
        if st.stopped.load(Ordering::Relaxed) {
            st.finished.store(true, Ordering::Relaxed);
            return None;
        }
        if st.paused.load(Ordering::Relaxed) {
            return Some(0.0);
        }
        if self.pcm.start >= self.pcm.end {
            match self.pcm.loop_start {
                // an empty loop would never yield a sample
                Some(l) if l < self.pcm.end => self.pcm.start = l,
                _ => {
                    st.finished.store(true, Ordering::Relaxed);
                    return None;
                }
            }
        }
        let s = self.pcm.sample(self.pcm.start);
        self.pcm.start += 1;
        Some(s * f32::from_bits(st.gain.load(Ordering::Relaxed)))
    }
}

impl Source for PcmSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        NonZero::new(1).unwrap()
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        NonZero::new(self.pcm.rate.max(1)).unwrap()
    }

    fn total_duration(&self) -> Option<Duration> {
        if self.pcm.loop_start.is_some() {
            None
        } else {
            Some(self.pcm.duration())
        }
    }
}

impl Drop for PcmSource {
    /// If the mixer drops the source early (device gone), it still counts
    /// as finished, so nobody waits for speech that will never end.
    fn drop(&mut self) {
        self.handle.0.finished.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(data: &[u8], loop_start: Option<usize>) -> (PcmSource, Handle) {
        let h = Handle::new(1.0);
        let data: Arc<[u8]> = data.into();
        let end = data.len();
        (PcmSource { pcm: Pcm { data, start: 0, end, rate: 11025, loop_start }, handle: h.clone() }, h)
    }

    #[test]
    fn plays_once_then_finishes() {
        let (mut s, h) = src(&[128, 255, 0], None);
        assert!(h.is_active());
        assert_eq!(s.next(), Some(0.0));
        assert_eq!(s.next(), Some(127.0 / 128.0));
        assert_eq!(s.next(), Some(-1.0));
        assert_eq!(s.next(), None);
        assert!(!h.is_active());
    }

    #[test]
    fn loops_tail_and_pauses_and_stops() {
        let (mut s, h) = src(&[1, 2, 3], Some(1));
        let got: Vec<f32> = (0..6).map(|_| s.next().unwrap()).collect();
        let want: Vec<f32> = [1u8, 2, 3, 2, 3, 2].iter().map(|&b| (b as f32 - 128.0) / 128.0).collect();
        assert_eq!(got, want);
        h.set_paused(true);
        assert_eq!(s.next(), Some(0.0));
        assert!(h.is_active());
        h.stop();
        assert!(!h.is_active());
        assert_eq!(s.next(), None);
    }

    /// Through a real rodio mixer, as the device would pull it: resampled
    /// from the sound's rate (here the 128 Hz some effects have) to the
    /// device's, and finished once played.
    #[test]
    fn plays_through_a_mixer() {
        let (mixer, mut out) = rodio::mixer::mixer(NonZero::new(2).unwrap(), NonZero::new(48000).unwrap());
        let data: Arc<[u8]> = vec![255u8; 128].into();
        let h = play_on(&mixer, Pcm::once(data, 128), 0.5);
        let mut n = 0;
        let mut peak = 0f32;
        while let Some(s) = out.next() {
            peak = peak.max(s);
            n += 1;
            assert!(n < 200_000, "never ends");
        }
        // one second of stereo at 48 kHz, give or take the resampler's edges
        assert!((95_000..=97_000).contains(&n), "{n} samples");
        assert!((peak - 0.5 * 127.0 / 128.0).abs() < 0.01, "peak {peak}");
        assert!(!h.is_active());
    }

    #[test]
    fn silent_audio_plays_nothing() {
        let a = Audio::silent();
        assert!(!a.available());
        assert!(a.play(Pcm::once(Arc::from(vec![128u8; 10]), 11025), 1.0).is_none());
        a.add_source(rodio::source::Zero::new(NonZero::new(1).unwrap(), NonZero::new(11025).unwrap()));
    }
}
