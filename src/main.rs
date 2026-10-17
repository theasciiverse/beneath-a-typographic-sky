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

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("dump") => tools::dump(&data_dir(), &args[1..]),
        Some("look") => tools::look(&data_dir(), &args[1..]),
        Some("music") => music_tool::music(&data_dir(), &args[1..]),
        _ => {
            eprintln!("usage: typosky dump <files|cpts|screens|rooms> [out dir]\n       typosky look <room> <braille|half> <cols> <rows> [out dir]\n       typosky music <section> <song> [seconds] [out.wav]");
            std::process::exit(2);
        }
    }
}
