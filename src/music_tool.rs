//! `typosky music <section> <song> [seconds] [out.wav]`: renders a song
//! of the AdLib music to a WAV file, or plays it when no file is given.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use crate::engine::disk::Disk;
use crate::engine::music::MusicHandle;

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

pub fn music(dir: &Path, args: &[String]) {
    let num = |i: usize, name: &str| -> Option<u32> {
        args.get(i).map(|s| s.parse().unwrap_or_else(|_| fail(&format!("bad {name}: {s}"))))
    };
    let (Some(section), Some(song)) = (num(0, "section"), num(1, "song")) else {
        fail("usage: typosky music <section> <song> [seconds] [out.wav]");
    };
    let seconds = num(2, "seconds").unwrap_or(30);
    let disk = Disk::open(dir).unwrap_or_else(|e| fail(&format!("{}: {e}", dir.display())));

    let music = MusicHandle::new();
    music.load_section(&disk, section as u8);
    let songs = music.song_count();
    if song == 0 || song > songs as u32 {
        fail(&format!("section {section} has songs 1 to {songs}"));
    }
    music.start_music(song as u16);

    match args.get(3) {
        Some(out) => {
            let rate = music.sample_rate();
            let mut samples = vec![0i16; (rate * seconds) as usize];
            music.render(&mut samples);
            write_wav(Path::new(out), rate, &samples).unwrap_or_else(|e| fail(&format!("{out}: {e}")));
            let peak = samples.iter().map(|s| s.saturating_abs()).max().unwrap_or(0);
            let rms = (samples.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / samples.len() as f64).sqrt();
            println!(
                "section {section} song {song} of {songs}: {seconds} s to {out}, peak {peak}, rms {rms:.0}, still playing: {}",
                music.music_is_playing()
            );
        }
        None => {
            let mut sink = rodio::DeviceSinkBuilder::open_default_sink()
                .unwrap_or_else(|e| fail(&format!("no audio device: {e}")));
            sink.log_on_drop(false);
            sink.mixer().add(music.source());
            println!("section {section} song {song} of {songs}, {seconds} s");
            std::thread::sleep(std::time::Duration::from_secs(seconds as u64));
        }
    }
}

/// A 16-bit mono PCM WAV file.
pub fn write_wav(path: &Path, rate: u32, samples: &[i16]) -> io::Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    let data_len = (samples.len() * 2) as u32;
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_len).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?; // PCM
    w.write_all(&1u16.to_le_bytes())?; // mono
    w.write_all(&rate.to_le_bytes())?;
    w.write_all(&(rate * 2).to_le_bytes())?; // bytes a second
    w.write_all(&2u16.to_le_bytes())?; // bytes a frame
    w.write_all(&16u16.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data_len.to_le_bytes())?;
    for s in samples {
        w.write_all(&s.to_le_bytes())?;
    }
    w.flush()
}
