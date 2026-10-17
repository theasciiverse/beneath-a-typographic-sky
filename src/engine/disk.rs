//! sky.dnr (file table) and sky.dsk (file data) reader. Port of ScummVM's
//! engines/sky/disk.cpp, without the iBASS branch.

use std::fs;
use std::io;
use std::path::Path;

use super::rnc::{self, Unpack};

/// Size of the 11-word header that starts every graphics file.
pub const HEADER_SIZE: usize = 22;

pub struct Disk {
    dnr: Vec<u8>,
    entries: usize,
    dsk: Vec<u8>,
    pub game_version: u32,
}

/// One file as loaded: its bytes, with the header (if it has one) still in front.
pub struct Loaded {
    pub data: Vec<u8>,
}

impl Disk {
    pub fn open(dir: &Path) -> io::Result<Disk> {
        let dnr = fs::read(dir.join("sky.dnr"))?;
        let dsk = fs::read(dir.join("sky.dsk"))?;
        if dnr.len() < 4 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "sky.dnr too short"));
        }
        let entries = u32::from_le_bytes([dnr[0], dnr[1], dnr[2], dnr[3]]) as usize;
        if dnr.len() < 4 + entries * 8 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "sky.dnr truncated"));
        }
        let game_version = match entries {
            232 => 272,
            243 => 109,
            247 => 267,
            1404 => 288,
            1413 => 303,
            1445 => {
                if dsk.len() == 8_830_435 {
                    348
                } else {
                    331
                }
            }
            1711 => 365,
            5099 => 368,
            5097 => 372,
            n => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unknown game version, {n} dinner table entries"),
                ))
            }
        };
        Ok(Disk { dnr, entries, dsk, game_version })
    }

    pub fn is_cd(&self) -> bool {
        matches!(self.game_version, 365 | 368 | 372)
    }

    pub fn is_demo(&self) -> bool {
        matches!(self.game_version, 109 | 267 | 272 | 365)
    }

    fn file_info(&self, file_nr: u16) -> Option<&[u8]> {
        (0..self.entries)
            .map(|i| &self.dnr[4 + i * 8..4 + i * 8 + 8])
            .find(|e| u16::from_le_bytes([e[0], e[1]]) == file_nr)
    }

    pub fn exists(&self, file_nr: u16) -> bool {
        self.file_info(file_nr).is_some()
    }

    /// All file numbers in the table, in table order.
    pub fn file_numbers(&self) -> Vec<u16> {
        (0..self.entries)
            .map(|i| u16::from_le_bytes([self.dnr[4 + i * 8], self.dnr[5 + i * 8]]))
            .collect()
    }

    pub fn load(&self, file_nr: u16) -> Option<Vec<u8>> {
        let info = self.file_info(file_nr)?;
        let le24 = |b: &[u8]| b[0] as u32 | (b[1] as u32) << 8 | (b[2] as u32) << 16;
        let flags = le24(&info[5..8]);
        let size = (flags & 0x03F_FFFF) as usize;
        let mut offset = le24(&info[2..5]);
        let shifted = (offset >> 23) & 1 != 0;
        offset &= 0x7F_FFFF;
        if shifted {
            offset <<= if self.game_version == 331 { 3 } else { 4 };
        }
        let offset = offset as usize;
        let end = (offset + size).min(self.dsk.len());
        if offset > end {
            log::warn!("file {file_nr} lies outside sky.dsk");
            return None;
        }
        let raw = &self.dsk[offset..end];
        if raw.len() != size {
            log::warn!("file {file_nr}: read {} of {size} bytes", raw.len());
        }

        // bit 23 set means stored uncompressed
        let uncompressed = (flags >> 23) & 1 != 0;
        let hdr_flag = if raw.len() >= 2 { u16::from_le_bytes([raw[0], raw[1]]) } else { 0 };
        if uncompressed || (hdr_flag >> 7) & 1 == 0 || raw.len() < HEADER_SIZE {
            return Some(raw.to_vec());
        }

        let tot_size = u16::from_le_bytes([raw[12], raw[13]]) as u32;
        let decomp_size = (((hdr_flag as u32) & !0xFF) << 8) | tot_size;
        let without_header = (flags >> 22) & 1 != 0;
        match rnc::unpack_m1(&raw[HEADER_SIZE..]) {
            Unpack::Ok(body) => {
                let mut out = Vec::with_capacity(body.len() + HEADER_SIZE);
                if !without_header {
                    out.extend_from_slice(&raw[..HEADER_SIZE]);
                }
                out.extend_from_slice(&body);
                if out.len() != decomp_size as usize {
                    log::debug!(
                        "file {file_nr}: decompressed {} bytes, header says {decomp_size}",
                        out.len()
                    );
                }
                Some(out)
            }
            Unpack::NotPacked => Some(raw.to_vec()),
            Unpack::PackedCrc => {
                log::warn!("file {file_nr}: packed CRC mismatch");
                None
            }
            Unpack::UnpackedCrc => {
                log::warn!("file {file_nr}: unpacked CRC mismatch");
                None
            }
        }
    }
}

/// The 11-word header in front of sprite and screen files.
#[derive(Clone, Copy, Debug, Default)]
pub struct Header {
    pub flag: u16,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub sp_size: u16,
    pub tot_size: u16,
    pub n_sprites: u16,
    pub offset_x: i16,
    pub offset_y: i16,
    pub compressed_size: u16,
}

impl Header {
    pub fn parse(d: &[u8]) -> Header {
        let w = |i: usize| u16::from_le_bytes([d[i * 2], d[i * 2 + 1]]);
        Header {
            flag: w(0),
            x: w(1),
            y: w(2),
            width: w(3),
            height: w(4),
            sp_size: w(5),
            tot_size: w(6),
            n_sprites: w(7),
            offset_x: w(8) as i16,
            offset_y: w(9) as i16,
            compressed_size: w(10),
        }
    }
}
