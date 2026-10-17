//! sky.cpt: the compacts (game objects, scripts' data, palettes, draw lists).
//! Port of ScummVM's engines/sky/compact.cpp.
//!
//! All compacts live in one word buffer, as in the original. A compact is
//! addressed by its id (list << 12 | index), which maps to a start word in
//! that buffer; several ids may alias the same start ("dlincs").

use std::fs;
use std::io;
use std::path::Path;

const SKY_CPT_SIZE: usize = 419_427;

pub const CPT_NULL: u16 = 0;
pub const COMPACT: u16 = 1;
pub const TURNTAB: u16 = 2;
pub const ANIMSEQ: u16 = 3;
pub const MISCBIN: u16 = 4;
pub const GETTOTAB: u16 = 5;
pub const ROUTEBUF: u16 = 6;
pub const MAINLIST: u16 = 7;

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub start: usize,
    pub size: u16,
    pub kind: u16,
}

pub struct Cpt {
    /// Every compact's words, back to back.
    pub raw: Vec<u16>,
    lists: Vec<Vec<Option<Entry>>>,
    names: Vec<Vec<Option<String>>>,
    pub save_ids: Vec<u16>,
    reset_data: Vec<u16>,
    reset_diffs: Vec<(u16, Vec<(u16, u16)>)>,
}

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn u16(&mut self) -> io::Result<u16> {
        let b = self
            .d
            .get(self.p..self.p + 2)
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "sky.cpt truncated"))?;
        self.p += 2;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> io::Result<u32> {
        let lo = self.u16()? as u32;
        let hi = self.u16()? as u32;
        Ok(lo | hi << 16)
    }
}

impl Cpt {
    pub fn open(dir: &Path, game_version: u32) -> io::Result<Cpt> {
        let data = fs::read(dir.join("sky.cpt"))?;
        if data.len() != SKY_CPT_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("sky.cpt is {} bytes, expected {SKY_CPT_SIZE}", data.len()),
            ));
        }
        let mut r = Reader { d: &data, p: 0 };
        if r.u16()? != 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown sky.cpt version"));
        }
        let num_lists = r.u16()? as usize;
        let mut list_len = Vec::with_capacity(num_lists);
        for _ in 0..num_lists {
            list_len.push(r.u16()? as usize);
        }
        let raw_words = r.u32()? as usize;
        let src_words = r.u32()? as usize;
        let src_start = r.p;
        r.p += src_words * 2;
        let ascii_size = r.u32()? as usize;
        let ascii = &data[r.p..r.p + ascii_size];
        r.p += ascii_size;
        let mut ascii_pos = 0usize;
        let mut next_name = || {
            let end = ascii[ascii_pos..].iter().position(|&c| c == 0).unwrap_or(0) + ascii_pos;
            let s = String::from_utf8_lossy(&ascii[ascii_pos..end]).into_owned();
            ascii_pos = end + 1;
            s
        };

        let mut raw = Vec::with_capacity(raw_words);
        let mut lists = Vec::with_capacity(num_lists);
        let mut names = Vec::with_capacity(num_lists);
        let mut src = Reader { d: &data, p: src_start };
        for &len in &list_len {
            let mut list = Vec::with_capacity(len);
            let mut lnames = Vec::with_capacity(len);
            for _ in 0..len {
                let size = src.u16()?;
                if size != 0 {
                    let kind = src.u16()?;
                    let start = raw.len();
                    for _ in 0..size {
                        raw.push(src.u16()?);
                    }
                    list.push(Some(Entry { start, size, kind }));
                    lnames.push(Some(next_name()));
                } else {
                    list.push(None);
                    lnames.push(None);
                }
            }
            lists.push(list);
            names.push(lnames);
        }

        let num_dlincs = r.u16()?;
        for _ in 0..num_dlincs {
            let dlinc = r.u16()?;
            let dest = r.u16()?;
            let (dl, di) = ((dlinc >> 12) as usize, (dlinc & 0xFFF) as usize);
            let target = lists[(dest >> 12) as usize][(dest & 0xFFF) as usize];
            lists[dl][di] = target;
            names[dl][di] = Some(next_name());
        }

        let num_diffs = r.u16()?;
        let diff_size = r.u16()? as usize;
        let mut diffs = Vec::with_capacity(diff_size);
        for _ in 0..diff_size {
            diffs.push(r.u16()?);
        }

        let num_save = r.u16()?;
        let mut save_ids = Vec::with_capacity(num_save as usize);
        for _ in 0..num_save {
            save_ids.push(r.u16()?);
        }

        let reset_words = r.u16()? as usize;
        let mut reset_data = Vec::with_capacity(reset_words);
        for _ in 0..reset_words {
            reset_data.push(r.u16()?);
        }
        let n_reset_diffs = r.u16()?;
        let mut reset_diffs = Vec::new();
        for _ in 0..n_reset_diffs {
            let version = r.u16()?;
            let fields = r.u16()?;
            let mut v = Vec::with_capacity(fields as usize);
            for _ in 0..fields {
                let pos = r.u16()?;
                let val = r.u16()?;
                v.push((pos, val));
            }
            reset_diffs.push((version, v));
        }

        let mut cpt = Cpt { raw, lists, names, save_ids, reset_data, reset_diffs };

        if game_version == 288 {
            let mut p = 0;
            for _ in 0..num_diffs {
                let id = diffs[p];
                let off = diffs[p + 1] as usize;
                let len = diffs[p + 2] as usize;
                p += 3;
                let start = cpt.start(id) + off;
                cpt.raw[start..start + len].copy_from_slice(&diffs[p..p + len]);
                p += len;
            }
        }

        cpt.fix_officer_blunt();
        Ok(cpt)
    }

    /// ScummVM bug #2687: an early sky.cpt pointed one talk animation at the
    /// wrong id, and Officer Blunt jumped into the lake for good.
    fn fix_officer_blunt(&mut self) {
        use super::defs::{CPT_TALK_TABLE_LIST, ID_SC31_GUARD_TALK, ID_SC31_GUARD_TALK2};
        let t = self.start(CPT_TALK_TABLE_LIST as u16) + 158;
        if self.raw[t] == ID_SC31_GUARD_TALK as u16 {
            self.raw[t] = ID_SC31_GUARD_TALK2 as u16;
        }
    }

    pub fn entry(&self, id: u16) -> Option<Entry> {
        self.lists
            .get((id >> 12) as usize)?
            .get((id & 0xFFF) as usize)
            .copied()
            .flatten()
    }

    /// Start word of compact `id` in `raw`. Panics on an id that does not
    /// exist, like the original's assert.
    pub fn start(&self, id: u16) -> usize {
        match self.entry(id) {
            Some(e) => e.start,
            None => panic!("compact {id:#06x} does not exist"),
        }
    }

    pub fn exists(&self, id: u16) -> bool {
        self.entry(id).is_some()
    }

    pub fn name(&self, id: u16) -> &str {
        self.names
            .get((id >> 12) as usize)
            .and_then(|l| l.get((id & 0xFFF) as usize))
            .and_then(|n| n.as_deref())
            .unwrap_or("(null)")
    }

    pub fn num_lists(&self) -> usize {
        self.lists.len()
    }

    pub fn list_len(&self, list: usize) -> usize {
        self.lists.get(list).map_or(0, |l| l.len())
    }

    pub fn find(&self, name: &str) -> Option<u16> {
        for (l, list) in self.names.iter().enumerate() {
            for (i, n) in list.iter().enumerate() {
                if n.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name)) {
                    return Some(((l << 12) | i) as u16);
                }
            }
        }
        None
    }

    /// The compact's words as a byte buffer (palettes are stored this way).
    pub fn bytes(&self, id: u16) -> Vec<u8> {
        let e = self.entry(id).expect("compact exists");
        self.raw[e.start..e.start + e.size as usize]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect()
    }

    /// Initial state of every saved compact, for a new game.
    pub fn reset_data(&self, game_version: u32) -> Option<Vec<u16>> {
        let (_, diff) = self.reset_diffs.iter().find(|(v, _)| *v as u32 == game_version)?;
        let mut data = self.reset_data.clone();
        for &(pos, val) in diff {
            data[pos as usize] = val;
        }
        Some(data)
    }
}
