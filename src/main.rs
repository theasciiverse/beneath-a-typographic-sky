mod audio;
mod audio_tools;
mod engine;
mod music_tool;
mod term;
mod tools;

use std::path::PathBuf;

fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("TYPOSKY_DATA") {
        return PathBuf::from(d);
    }
    PathBuf::from("reference/bass-cd-1.2")
}

fn play(args: &[String]) {
    let dir = data_dir();
    if !dir.join("sky.dsk").exists() {
        eprintln!("typosky: no game data in {} (set TYPOSKY_DATA)", dir.display());
        std::process::exit(1);
    }
    let host = match term::app::TermHost::new() {
        Ok(mut h) => {
            h.audio = !args.iter().any(|a| a == "--mute");
            h.splash(
                &[
                    "BENEATH A TYPOGRAPHIC SKY",
                    "",
                    "a modified version of Beneath a Steel Sky",
                    "(C) Revolution Software Ltd, freeware since 2003",
                    "",
                    "engine ported from ScummVM  .  an Asciiverse restoration",
                    "",
                    "left click looks  .  right click uses  .  pointer to the top for the inventory",
                    "F5 options  .  Esc skips  .  F2 switches Braille and blocks",
                ],
                4000,
            );
            h
        }
        Err(e) => {
            eprintln!("typosky: {e}");
            std::process::exit(1);
        }
    };
    let mut sky = match engine::sky::Sky::new(&dir, Box::new(host)) {
        Ok(s) => s,
        Err(e) => {
            ratatui::restore();
            eprintln!("typosky: {e}");
            std::process::exit(1);
        }
    };
    sky.screen.text_pixels = false;
    sky.front_end = true;
    let skip_intro = args.iter().any(|a| a == "--skip-intro");
    sky.run(skip_intro);
}

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("dump") => tools::dump(&data_dir(), &args[1..]),
        Some("look") => tools::look(&data_dir(), &args[1..]),
        Some("headless") => tools::headless(&data_dir(), &args[1..]),
        Some("probe") => tools::probe(&data_dir(), &args[1..]),
        Some("text") => tools::text(&data_dir(), &args[1..]),
        Some("monkey") => tools::monkey(&data_dir(), &args[1..]),
        Some("modes") => tools::modes(&args[1..]),
        None | Some("play") | Some("--skip-intro") | Some("--mute") => play(&args),
        Some("sfx") => audio_tools::sfx(&data_dir(), &args[1..]),
        Some("speech") => audio_tools::speech(&data_dir(), &args[1..]),
        Some("music") => music_tool::music(&data_dir(), &args[1..]),
        _ => {
            eprintln!(
                "usage: typosky [play] [--skip-intro] [--mute]
       typosky dump <files|cpts|screens|rooms> [out dir]
       typosky look <room> <braille|half> <cols> <rows> [out dir]
       typosky headless [--intro|--floppy] [--every-ms=N] [--skip-at=N] <cycles> [every] [out dir] [cycle:x,y[,button] ...]
       typosky text <number> ...
       typosky sfx <fx 256-394 | section:sound> [room] [--wav out.wav]
       typosky speech <text number> [--wav out.wav]
       typosky music <section> <song> [seconds] [out.wav]"
            );
            std::process::exit(2);
        }
    }
}
