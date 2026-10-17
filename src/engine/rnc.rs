//! Rob Northen ProPack method 1 decompression, as used by sky.dsk.
//! Port of ScummVM's common/compression/rnc_deco.cpp.

const HEADER_LEN: usize = 18;
const MIN_LENGTH: u32 = 2;
const RNC1_SIGNATURE: u32 = 0x524E_4301;

pub enum Unpack {
    NotPacked,
    PackedCrc,
    UnpackedCrc,
    Ok(Vec<u8>),
}

struct Decoder<'a> {
    src: &'a [u8],
    pos: usize,
    bit_buff_l: u16,
    bit_buff_h: u16,
    bit_count: u8,
    input_left: i32,
    crc_table: [u16; 256],
    raw_table: [u16; 64],
    pos_table: [u16; 64],
    len_table: [u16; 64],
}

fn crc_table() -> [u16; 256] {
    let mut table = [0u16; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        let mut v = i as u16;
        for _ in 0..8 {
            if v & 1 != 0 {
                v = (v >> 1) ^ 0xA001;
            } else {
                v >>= 1;
            }
        }
        *entry = v;
    }
    table
}

fn crc_block(table: &[u16; 256], block: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in block {
        crc ^= b as u16;
        let tmp = crc >> 8;
        crc = table[(crc & 0xFF) as usize] ^ tmp;
    }
    crc
}

impl<'a> Decoder<'a> {
    fn byte(&self, at: usize) -> u16 {
        self.src.get(at).copied().unwrap_or(0) as u16
    }

    fn le16(&self, at: usize) -> u16 {
        self.byte(at) | (self.byte(at + 1) << 8)
    }

    fn input_bits(&mut self, mut amount: u8) -> u16 {
        let mut h = self.bit_buff_h as u32;
        let mut l = self.bit_buff_l as u32;
        let mut count = self.bit_count as i16;
        let ret = (((1u32 << amount) - 1) & l) as u16;
        count -= amount as i16;
        if count < 0 {
            count += amount as i16;
            let rem = (h << (16 - count as u32)) & 0xFFFF;
            h >>= count as u32;
            l >>= count as u32;
            l |= rem;
            self.pos += 2;
            self.input_left -= 2;
            h = if self.input_left <= 0 {
                0
            } else if self.input_left == 1 {
                self.byte(self.pos) as u32
            } else {
                self.le16(self.pos) as u32
            };
            amount -= count as u8;
            count = 16 - amount as i16;
        }
        let rem = (h << (16 - amount as u32)) & 0xFFFF;
        self.bit_buff_h = (h >> amount) as u16;
        self.bit_buff_l = ((l >> amount) | rem) as u16;
        self.bit_count = count as u8;
        ret
    }

    fn make_huff_table(&mut self, which: u8) {
        let num_codes = self.input_bits(5);
        if num_codes == 0 {
            return;
        }
        let mut huff_len = [0u8; 16];
        for len in huff_len.iter_mut().take(num_codes as usize) {
            *len = (self.input_bits(4) & 0xFF) as u8;
        }
        let mut table = [0u16; 64];
        let mut t = 0usize;
        let mut huff_code: u16 = 0;
        for bit_len in 1u16..17 {
            for i in 0..num_codes as usize {
                if huff_len[i] as u16 == bit_len {
                    table[t] = ((1u32 << bit_len) - 1) as u16;
                    let b = (huff_code >> (16 - bit_len)) as u16;
                    let mut a: u16 = 0;
                    for j in 0..bit_len {
                        a |= ((b >> j) & 1) << (bit_len - j - 1);
                    }
                    table[t + 1] = a;
                    table[t + 2 + 0x1E] = ((huff_len[i] as u16) << 8) | (i as u16 & 0xFF);
                    t += 2;
                    huff_code = huff_code.wrapping_add((1u32 << (16 - bit_len)) as u16);
                }
            }
        }
        match which {
            0 => self.raw_table = table,
            1 => self.pos_table = table,
            _ => self.len_table = table,
        }
    }

    fn input_value(&mut self, which: u8) -> u16 {
        let table = match which {
            0 => self.raw_table,
            1 => self.pos_table,
            _ => self.len_table,
        };
        let value = self.bit_buff_l;
        let mut t = 0usize;
        loop {
            let two = table[t] & value;
            let one = table[t + 1];
            t += 2;
            if one == two || t >= 32 {
                break;
            }
        }
        let mut value = table[t + 0x1E];
        self.input_bits(((value >> 8) & 0xFF) as u8);
        value &= 0xFF;
        if value >= 2 {
            value -= 1;
            let mut v = self.input_bits(value as u8);
            v |= 1 << value;
            value = v;
        }
        value
    }
}

/// Unpacks an "RNC\x01" stream. `input` starts at the RNC signature.
pub fn unpack_m1(input: &[u8]) -> Unpack {
    let be32 = |at: usize| -> u32 {
        input
            .get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
            .unwrap_or(0)
    };
    let be16 = |at: usize| -> u16 {
        input
            .get(at..at + 2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
            .unwrap_or(0)
    };
    if be32(0) != RNC1_SIGNATURE {
        return Unpack::NotPacked;
    }
    let unpack_len = be32(4) as usize;
    let pack_len = be32(8) as usize;
    let crc_unpacked = be16(12);
    let crc_packed = be16(14);
    let blocks = input.get(17).copied().unwrap_or(0);
    let crc = crc_table();
    let packed = match input.get(HEADER_LEN..HEADER_LEN + pack_len) {
        Some(p) => p,
        None => return Unpack::PackedCrc,
    };
    if crc_block(&crc, packed) != crc_packed {
        return Unpack::PackedCrc;
    }

    let mut d = Decoder {
        src: input,
        pos: HEADER_LEN,
        bit_buff_l: 0,
        bit_buff_h: 0,
        bit_count: 0,
        input_left: input.len() as i32 - HEADER_LEN as i32,
        crc_table: crc,
        raw_table: [0; 64],
        pos_table: [0; 64],
        len_table: [0; 64],
    };
    let mut out: Vec<u8> = Vec::with_capacity(unpack_len);
    d.bit_buff_l = d.le16(d.pos);
    d.input_bits(2);

    let mut blocks = blocks;
    loop {
        d.make_huff_table(0);
        d.make_huff_table(1);
        d.make_huff_table(2);
        let mut counts = d.input_bits(16);
        loop {
            let len = d.input_value(0) as u32;
            if len != 0 {
                if d.input_left < len as i32 {
                    return Unpack::NotPacked;
                }
                let end = d.pos + len as usize;
                if end > input.len() {
                    return Unpack::NotPacked;
                }
                out.extend_from_slice(&input[d.pos..end]);
                d.pos = end;
                d.input_left -= len as i32;
                let a: u16 = if d.input_left <= 0 {
                    0
                } else if d.input_left == 1 {
                    d.byte(d.pos)
                } else {
                    d.le16(d.pos)
                };
                let b: u16 = if d.input_left <= 2 {
                    0
                } else if d.input_left == 3 {
                    d.byte(d.pos + 2)
                } else {
                    d.le16(d.pos + 2)
                };
                let bc = d.bit_count as u32;
                let mut l = d.bit_buff_l as u32;
                l &= (1u32 << bc) - 1;
                l |= (a as u32) << bc;
                d.bit_buff_l = l as u16;
                d.bit_buff_h = (((a as u32) >> (16 - bc)) | ((b as u32) << bc)) as u16;
            }
            if counts > 1 {
                let offset = d.input_value(1) as usize + 1;
                let length = d.input_value(2) as u32 + MIN_LENGTH;
                if offset > out.len() {
                    return Unpack::NotPacked;
                }
                let mut from = out.len() - offset;
                for _ in 0..length {
                    let v = out[from];
                    out.push(v);
                    from += 1;
                }
            }
            counts = counts.wrapping_sub(1);
            if counts == 0 {
                break;
            }
        }
        blocks = blocks.wrapping_sub(1);
        if blocks == 0 {
            break;
        }
    }
    out.truncate(unpack_len);
    if crc_block(&d.crc_table, &out) != crc_unpacked {
        return Unpack::UnpackedCrc;
    }
    Unpack::Ok(out)
}
