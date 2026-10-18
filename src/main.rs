mod audio;
mod audio_tools;
mod engine;
mod music_tool;
mod term;
mod tools;
mod walk;

use std::path::PathBuf;

/// Where the game data (sky.dsk, sky.dnr, sky.cpt) is looked for, in
/// order: TYPOSKY_DATA; data/ beside the binary (the release archive);
/// beside the binary; reference/bass-cd-1.2 (a development checkout).
fn data_candidates() -> Vec<PathBuf> {
    let mut c = Vec::new();
    if let Some(d) = std::env::var_os("TYPOSKY_DATA") {
        c.push(PathBuf::from(d));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()).and_then(|e| e.parent().map(PathBuf::from)) {
        c.push(dir.join("data"));
        c.push(dir);
    }
    c.push(PathBuf::from("reference/bass-cd-1.2"));
    c
}

fn data_dir() -> PathBuf {
    let c = data_candidates();
    c.iter().find(|d| d.join("sky.dsk").is_file()).cloned().unwrap_or_else(|| c[0].clone())
}

fn play(args: &[String]) {
    let dir = data_dir();
    if !dir.join("sky.dsk").exists() {
        eprintln!("typosky: game data (sky.dsk, sky.dnr, sky.cpt) not found; looked in:");
        for d in data_candidates() {
            eprintln!("  {}", d.display());
        }
        eprintln!("set TYPOSKY_DATA to the folder that has them");
        std::process::exit(1);
    }
    let host = match term::app::TermHost::new() {
        Ok(mut h) => {
            h.audio = !args.iter().any(|a| a == "--mute");
            h.game_font = args.iter().any(|a| a == "--game-font");
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
    sky.screen.text_pixels = args.iter().any(|a| a == "--game-font");
    sky.front_end = true;
    sky.init_language();
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
        Some("walk") => walk::walk(&data_dir(), &args[1..]),
        Some("modes") => tools::modes(&args[1..]),
        Some("glyphs") => tools::glyphs(&data_dir(), &args[1..]),
        Some("trace") => tools::trace(&data_dir(), &args[1..]),
        None | Some("play") | Some("--skip-intro") | Some("--mute") | Some("--game-font") => play(&args),
        Some("sfx") => audio_tools::sfx(&data_dir(), &args[1..]),
        Some("speech") => audio_tools::speech(&data_dir(), &args[1..]),
        Some("music") => music_tool::music(&data_dir(), &args[1..]),
        _ => {
            eprintln!(
                "usage: typosky [play] [--skip-intro] [--mute] [--game-font]
       typosky dump <files|cpts|screens|rooms> [out dir]
       typosky look <room> <braille|half> <cols> <rows> [out dir]
       typosky headless [--intro|--floppy] [--every-ms=N] [--skip-at=N] <cycles> [every] [out dir] [cycle:x,y[,button] ...]
       typosky text <number> ...
       typosky walk <script> [--until-linc] [--load=save] [--from=line] [--save-at=line:slot] [--frames=dir] [--saves=dir]
       typosky sfx <fx 256-394 | section:sound> [room] [--wav out.wav]
       typosky speech <text number> [--wav out.wav]
       typosky music <section> <song> [seconds] [out.wav]"
            );
            std::process::exit(2);
        }
    }
}
