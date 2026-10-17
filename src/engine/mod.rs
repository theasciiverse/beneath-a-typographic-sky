//! The Beneath a Steel Sky engine, ported from ScummVM's engines/sky.
//! It produces a 320x200 8-bit frame and a palette; drawing that frame in
//! the terminal is the job of `crate::term`.

pub mod audio_glue;
pub mod autoroute;
pub mod cache;
pub mod control;
pub mod cpt;
pub mod defs;
pub mod disk;
pub mod grid;
pub mod host;
pub mod hufftext;
pub mod intro;
pub mod logic;
pub mod mcode;
pub mod mouse;
pub mod music;
pub mod rnc;
pub mod screen;
pub mod sky;
pub mod sound;
pub mod text;
