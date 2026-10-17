//! Listening checks: `typosky sfx ...` and `typosky speech ...` play one
//! effect or one spoken line on the default device and wait for it to end,
//! or write it to a WAV file.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::audio::{Audio, Pcm};
use crate::engine::defs::{SF_ALLOW_SPEECH, SF_SBLASTER};
use crate::engine::disk::Disk;
use crate::engine::sound::{Sound, MAX_CHANNEL_VOLUME};

/// One game tick, as SkyEngine's `gameSpeed` of 80 ms.
const TICK: Duration = Duration::from_millis(80);

/// A looping effect never ends; stop listening after this long.
const MAX_LISTEN: Duration = Duration::from_secs(8);

fn fail(msg: &str) -> ! {
    eprintln!("typosky: {msg}");
    std::process::exit(1);
}

/// Splits off `--wav <path>`.
fn wav_arg(args: &[String]) -> (Vec<String>, Option<String>) {
    let mut rest = Vec::new();
    let mut wav = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--wav" {
            wav = Some(it.next().cloned().unwrap_or_else(|| fail("--wav needs a file name")));
        } else {
            rest.push(a.clone());
        }
    }
    (rest, wav)
}

fn num(s: &str) -> u32 {
    s.parse().unwrap_or_else(|_| fail(&format!("not a number: {s}")))
}

/// `typosky sfx <fx> [section] [room]`: an fx number from 256 as scripts
/// start it (in `room`, by default the first room it is listed for), or a
/// plain sound number below 256 of the section's file at full volume.
/// The section is the one `Sound::load_section` takes (Logic's section
/// plus one), 1 by default.
pub fn sfx(dir: &Path, args: &[String]) {
    let (args, wav) = wav_arg(args);
    let n = num(args.first().map(String::as_str).unwrap_or_else(|| fail("which sfx?")));
    let section = args.get(1).map(|s| num(s) as u8).unwrap_or(1);
    let disk = Disk::open(dir).unwrap_or_else(|e| fail(&format!("{}: {e}", dir.display())));
    let audio = if wav.is_some() { Audio::silent() } else { Audio::new() };
    let mut sound = Sound::new(&audio, MAX_CHANNEL_VOLUME);
    sound.load_section(&disk, section, disk.game_version, 0).unwrap_or_else(|e| fail(&e.to_string()));
    println!("section {section}: {} sounds", sound.sounds_total);

    let sound_no = if n >= 256 {
        let (sound_no, flags, rooms) = Sound::fx_entry(n).unwrap_or_else(|| fail("fx numbers go up to 394"));
        let room = args.get(2).map(|s| num(s)).unwrap_or(match rooms[0].0 {
            0xff => 0,
            r => r as u32,
        });
        println!("fx {n}: sound {sound_no}, flags {flags:02x}, rooms {rooms:?}, playing for room {room}");
        if wav.is_none() {
            sound.fn_start_fx(n, 0, room, SF_SBLASTER);
        }
        sound_no
    } else {
        if wav.is_none() {
            sound.play_sound(n as u16, 127, 0);
        }
        n as u8
    };

    let Some(s) = sound.sfx_sample(sound_no) else { fail(&format!("sound {sound_no} has no samples")) };
    println!(
        "sound {sound_no}: {} samples at {} Hz (table {}), {:.2} s, loop {}",
        s.pcm.len(),
        s.pcm.rate,
        s.table_rate,
        s.pcm.duration().as_secs_f32(),
        match s.pcm.loop_start {
            Some(l) => format!("from sample {}", l - s.pcm.start),
            None => "none".into(),
        }
    );
    describe(&s.pcm);
    if let Some(path) = wav {
        write_wav(Path::new(&path), &s.pcm);
        return;
    }
    if !audio.available() {
        fail("no audio device");
    }

    // Run game ticks so delayed effects start, until the effect ends.
    let t0 = Instant::now();
    let mut started = false;
    while t0.elapsed() < MAX_LISTEN {
        sound.check_fx_queue();
        let active = sound.fx_active(0);
        if started && !active {
            break;
        }
        started |= active;
        std::thread::sleep(TICK);
    }
}

/// `typosky speech <text number>`: one spoken line, as Logic starts it.
pub fn speech(dir: &Path, args: &[String]) {
    let (args, wav) = wav_arg(args);
    let text_num = num(args.first().map(String::as_str).unwrap_or_else(|| fail("which text number?"))) as u16;
    let disk = Disk::open(dir).unwrap_or_else(|e| fail(&format!("{}: {e}", dir.display())));
    let file = Sound::speech_file(text_num);
    let Some(pcm) = Sound::speech_pcm(&disk, text_num) else {
        fail(&format!("no speech for text {text_num} (file {file:?})"))
    };
    println!(
        "text {text_num} ({:#06x}): file {}, {} samples at {} Hz, {:.2} s",
        text_num,
        file.unwrap_or(0),
        pcm.len(),
        pcm.rate,
        pcm.duration().as_secs_f32()
    );
    describe(&pcm);
    if let Some(path) = wav {
        write_wav(Path::new(&path), &pcm);
        return;
    }

    let audio = Audio::new();
    if !audio.available() {
        fail("no audio device");
    }
    let mut sound = Sound::new(&audio, MAX_CHANNEL_VOLUME);
    if !sound.start_speech(&disk, text_num, SF_ALLOW_SPEECH) {
        fail("speech did not start");
    }
    // Ask every tick, as Logic does.
    while !sound.speech_finished() {
        std::thread::sleep(TICK);
    }
}

/// Prints the peak and RMS level, so a decode can be judged without ears.
fn describe(pcm: &Pcm) {
    let n = pcm.len().max(1) as f64;
    let (mut peak, mut sq) = (0f32, 0f64);
    for i in pcm.start..pcm.end {
        let s = pcm.sample(i);
        peak = peak.max(s.abs());
        sq += (s * s) as f64;
    }
    println!("peak {:.3}, rms {:.3}", peak, (sq / n).sqrt());
}

/// Writes the samples as they are: 8-bit unsigned mono PCM WAV.
fn write_wav(path: &Path, pcm: &Pcm) {
    let data = &pcm.data[pcm.start..pcm.end];
    let mut w = Vec::with_capacity(44 + data.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&pcm.rate.to_le_bytes());
    w.extend_from_slice(&pcm.rate.to_le_bytes()); // bytes per second
    w.extend_from_slice(&1u16.to_le_bytes()); // block align
    w.extend_from_slice(&8u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(data);
    fs::write(path, w).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())));
    println!("wrote {}", path.display());
}
