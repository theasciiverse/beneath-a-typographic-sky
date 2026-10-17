mod engine;
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
        Ok(h) => h,
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
        None | Some("play") | Some("--skip-intro") => play(&args),
        _ => {
            eprintln!("usage: typosky dump <files|cpts|screens|rooms> [out dir]\n       typosky look <room> <braille|half> <cols> <rows> [out dir]");
            std::process::exit(2);
        }
    }
}
