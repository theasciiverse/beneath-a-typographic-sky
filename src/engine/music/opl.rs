//! The OPL2 chip the AdLib driver writes to. Port of the parts of ScummVM's
//! audio/softsynth/opl/nuked.cpp (the `OPL::OPL` glue around Nuked-OPL3)
//! and audio/chip.cpp (`EmulatedChip`'s timer) that the sky engine uses.
//!
//! The synthesis itself is the `nuked-opl3` crate (Tony Gies' pure Rust
//! port of Nuke.YKT's Nuked-OPL3 1.8 by way of Nuked-OPL3-fast,
//! LGPL-2.1-or-later). ScummVM's own Nuked emulator is the same
//! Nuked-OPL3-fast 1.8 code in C, so the chip core matches ScummVM's
//! "nuked" OPL driver. ScummVM's default for OPL2 is its MAME emulator;
//! Nuked is the more accurate one.
//!
//! Like ScummVM with an OPL2 game, the chip runs in OPL2 mode (the OPL3
//! "new" bit is never set), where both output channels carry every voice;
//! they differ only by Nuked's emulation of the OPL3's small left/right
//! sample delay. We keep the left one and produce mono.

use nuked_opl3::Opl3Chip;

/// audio/chip.h: timer positions are 16.16 fixed point sample counts.
pub(super) const FIXP_SHIFT: u32 = 16;

pub struct Opl {
    chip: Opl3Chip,
    rate: u32,
    /// EmulatedChip::_nextTick: samples until the next timer callback, 16.16.
    pub(super) next_tick: i64,
    /// EmulatedChip::_samplesPerTick, 16.16.
    pub(super) samples_per_tick: i64,
    stereo: Vec<i16>,
    /// Samples produced so far, for the register log.
    samples: u64,
}

impl Opl {
    /// `OPL::OPL` plus `OPL::init`: a reset chip producing `rate` samples a second.
    pub fn new(rate: u32) -> Opl {
        Opl { chip: Opl3Chip::new(rate), rate, next_tick: 0, samples_per_tick: 0, stereo: Vec::new(), samples: 0 }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// `OPL::writeReg`: ScummVM's Nuked glue queues every write through
    /// the chip's write buffer, as a real chip needs time between writes.
    pub fn write_reg(&mut self, reg: u8, value: u8) {
        // `RUST_LOG=opl=trace` logs every write with its sample position,
        // enough to replay the music on another OPL emulator
        log::trace!(target: "opl", "{} {reg:02x} {value:02x}", self.samples);
        self.chip.write_register_buffered(reg as u16, value);
    }

    /// `EmulatedChip::setCallbackFrequency`.
    pub fn set_callback_frequency(&mut self, timer_frequency: u32) {
        let rate = self.rate as i64;
        let base = timer_frequency as i64;
        let d = rate / base;
        let r = rate % base;
        // This is equivalent to (getRate() << FIXP_SHIFT) / BASE_FREQ
        // but less prone to arithmetic overflow.
        self.samples_per_tick = (d << FIXP_SHIFT) + (r << FIXP_SHIFT) / base;
    }

    /// `OPL::generateSamples` (`OPL3_GenerateStream`), mono.
    pub fn generate_samples(&mut self, out: &mut [i16]) {
        if out.is_empty() {
            return;
        }
        self.stereo.resize(out.len() * 2, 0);
        // the buffer is never shorter than two samples here
        let _ = self.chip.generate_stream(&mut self.stereo);
        for (o, frame) in out.iter_mut().zip(self.stereo.chunks_exact(2)) {
            *o = frame[0];
        }
        self.samples += out.len() as u64;
    }
}
