//! The Beneath a Steel Sky engine, ported from ScummVM's engines/sky.
//! It produces a 320x200 8-bit frame and a palette; drawing that frame in
//! the terminal is the job of `crate::term`.

pub mod cpt;
pub mod defs;
pub mod disk;
pub mod rnc;
