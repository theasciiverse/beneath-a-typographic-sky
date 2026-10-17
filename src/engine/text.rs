//! Game text: Huffman-packed strings, word wrap, and text sprites drawn in
//! the game's bitmap fonts. Port of ScummVM's engines/sky/text.cpp.
//!
//! Each text sprite also keeps its words (`TextMeta`), so the terminal can
//! set them in real type instead of as pixels.

use super::defs::*;
use super::disk::HEADER_SIZE;
use super::hufftext::HUFF_TREE_372;
use super::screen::FIRST_TEXT_BUFFER;
use super::sky::{f, Sky};

const FIRST_TEXT_SEC: usize = 77;
const NO_OF_TEXT_SECTIONS: u32 = 8;
const CHAR_SET_FILE: u16 = 60150;
const CHAR_SET_HEADER: usize = 128;
const MAX_NO_LINES: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug)]
pub struct TextMeta {
    pub lines: Vec<String>,
    pub color: u8,
    pub centered: bool,
    pub line_height: u16,
}

#[derive(Default, Clone)]
struct CharSet {
    data: Vec<u8>,
    height: u8,
    spacing: u32,
}

#[derive(Default)]
pub struct TextState {
    main: CharSet,
    link: CharSet,
    control: CharSet,
    pub cur_char_set: u32,
    pub num_letters: u32,
    mouse_ofs_x: u32,
    mouse_ofs_y: u32,
    /// words of the text sprites in items 274..284
    pub meta: [Option<TextMeta>; 11],
}

pub struct DisplayedText {
    pub data: Vec<u8>,
    /// width of the last line
    pub width: u32,
    pub height: u16,
    pub lines: Vec<String>,
    pub compact_num: u16,
}

/// Patched messages, from ScummVM: translations missing in some versions.
const PATCHED: [(u32, &str); 9] = [
    (28724, "Testo e Parlato"),
    (28707, "Solo Testo"),
    (28693, "Solo Parlato"),
    (28724, "Text och tal"),
    (28707, "Endast text"),
    (28693, "Endast tal"),
    (28686, "Musikvolym"),
    (4336, "Wir befinden uns EINHUNDERTZWANZIG METER #ber dem ERBODEN!"),
    (28686, "Volume de musique"),
];
const PATCH_LANG_IDX: [u16; 8] = [0xFFFF, 7, 8, 0xFFFF, 3, 0, 0xFFFF, 0xFFFF];
const PATCH_LANG_NUM: [u16; 8] = [0, 1, 1, 0, 4, 3, 0, 0];

impl Sky {
    pub(crate) fn text_init(&mut self) {
        let main = self.disk.load(CHAR_SET_FILE).expect("main font");
        self.text.main = CharSet { data: main, height: MAIN_CHAR_HEIGHT as u8, spacing: 0 };
        if !self.disk.is_demo() {
            self.text.control = CharSet { data: self.disk.load(60520).unwrap_or_default(), height: 12, spacing: 0 };
            self.text.link = CharSet { data: self.disk.load(60521).unwrap_or_default(), height: 12, spacing: 1 };
        }
        self.fn_set_font(0);
    }

    pub(crate) fn fn_set_font(&mut self, font: u32) {
        assert!(font <= 2, "tried to set invalid font {font}");
        self.text.cur_char_set = font;
    }

    fn charset(&self) -> &CharSet {
        match self.text.cur_char_set {
            1 => &self.text.link,
            2 => &self.text.control,
            _ => &self.text.main,
        }
    }

    pub(crate) fn fn_text_module(&mut self, info_id: u32, text_no: u32) {
        self.fn_set_font(1);
        let msg = self.cpt_start(info_id);
        let (w, logic, x, y) = (self.w(msg + 1), self.w(msg + 2), self.w(msg + 3), self.w(msg + 4));
        let t = self.low_text_manager(text_no, w, logic, 209, Align::Left);
        self.vars[RESULT as usize] = t.compact_num as u32;
        let c = self.cpt_start(t.compact_num as u32);
        self.s(c, f::XCOOD, x);
        self.s(c, f::YCOOD, y);
        self.fn_set_font(0);
    }

    /// Text number `nr` as bytes (no terminating zero).
    pub fn get_text(&mut self, nr: u32) -> Vec<u8> {
        let lang = self.sys.language as usize;
        if lang < 8 {
            let (idx, num) = (PATCH_LANG_IDX[lang], PATCH_LANG_NUM[lang]);
            for i in 0..num {
                let (n, s) = PATCHED[(idx + i) as usize];
                if n == nr {
                    return s.as_bytes().to_vec();
                }
            }
        }

        let section = (nr & 0xF000) >> 12;
        let slot = FIRST_TEXT_SEC + section as usize;
        if self.items[slot].is_none() {
            let file = section + self.sys.language as u32 * NO_OF_TEXT_SECTIONS + 60600;
            self.items[slot] = self.disk.load(file as u16);
        }
        let Some(data) = self.items[slot].as_ref() else {
            return b"???".to_vec();
        };
        let byte = |at: usize| data.get(at).copied().unwrap_or(0);
        let le16 = |at: usize| u16::from_le_bytes([byte(at), byte(at + 1)]) as u32;

        let mut offset = 0u32;
        let block_nr = nr & 0xFE0;
        let mut msg = nr & 0x1F;
        if block_nr != 0 {
            let mut bp = 4usize;
            for _ in 0..block_nr >> 5 {
                offset += le16(bp);
                bp += 2;
            }
        }
        if msg != 0 {
            let mut bp = (block_nr + le16(0)) as usize;
            while msg > 0 {
                let mut skip = byte(bp) as u32;
                bp += 1;
                if skip & 0x80 != 0 {
                    skip &= 0x7F;
                    skip <<= 3;
                }
                offset += skip;
                msg -= 1;
            }
        }
        let mut bit_pos = offset & 3;
        offset >>= 2;
        offset += le16(2);
        let mut p = offset as usize;
        // bit pointer: 0->8, 1->6, 2->4 ...
        bit_pos ^= 3;
        bit_pos += 1;
        bit_pos <<= 1;

        let mut out = Vec::new();
        loop {
            // getTextChar
            let mut node = 0usize;
            let ch = loop {
                // getTextBit
                if bit_pos != 0 {
                    bit_pos -= 1;
                } else {
                    p += 1;
                    bit_pos = 7;
                }
                let bit = (data.get(p).copied().unwrap_or(0) >> bit_pos) & 1 != 0;
                let (l, r, _) = HUFF_TREE_372[node];
                node = if bit { r } else { l } as usize;
                let (l, r, v) = HUFF_TREE_372[node];
                if l == 0 && r == 0 {
                    break v;
                }
            };
            // a text number past the end of its section decodes zeros for
            // ever (ScummVM would read past the buffer); stop at the end
            if ch == 0 || p >= data.len() {
                break;
            }
            out.push(ch);
        }
        out
    }

    /// Text::displayText: lay out and draw `text` into a new text sprite.
    pub fn display_text(&mut self, mut text: Vec<u8>, align: Align, pixel_width: u16, color: u8) -> DisplayedText {
        // work around bug #1080 (line width exceeded)
        replace_all(&mut text, b"MUND-BEATMUNG!", b"MUND BEATMUNG!");
        // work around bug #1940
        replace_all(&mut text, b"MANIFESTACION-ARTISTICA.", b"MANIFESTACION ARTISTICA.");
        text.push(0);

        let cs = self.charset().clone();
        let at = |t: &Vec<u8>, i: usize| *t.get(i).unwrap_or(&0);
        let mut center = vec![0u32; MAX_NO_LINES + 2];
        let mut line_width: u32 = 0;
        let mut num_lines = 0usize;
        self.text.num_letters = 2;

        let mut cur = 0usize;
        let mut last_space = 0usize;
        let mut ch = at(&text, cur);
        cur += 1;
        while ch >= 0x20 {
            let old_width = line_width;
            let mut c = ch;
            if self.text.cur_char_set == 1 && c >= 0x80 {
                c = 0x20;
            }
            c -= 0x20;
            if c == 0 {
                last_space = cur; // keep track of the last space
                if num_lines < center.len() {
                    center[num_lines] = line_width;
                }
            }
            line_width += *cs.data.get(c as usize).unwrap_or(&0) as u32;
            line_width += cs.spacing;

            if pixel_width as u32 <= line_width {
                // no space to break at: break right here
                if last_space == 0 || at(&text, last_space - 1) == 10 {
                    cur = cur.saturating_sub(1);
                    text.insert(cur, 10);
                    last_space = cur + 1;
                    if num_lines < center.len() {
                        center[num_lines] = old_width;
                    }
                }
                text[last_space - 1] = 10;
                line_width = 0;
                num_lines += 1;
                cur = last_space; // go back for a new count
            }
            ch = at(&text, cur);
            cur += 1;
            self.text.num_letters += 1;
        }
        let last_width = line_width;
        if num_lines < center.len() {
            center[num_lines] = line_width;
        }
        num_lines += 1;
        if num_lines > MAX_NO_LINES {
            log::warn!("maximum number of text lines exceeded ({num_lines})");
        }

        let char_height = cs.height as usize;
        let line_size = pixel_width as usize * char_height;
        let num_bytes = line_size * num_lines + HEADER_SIZE + 4;
        let mut dest = vec![0u8; num_bytes];
        let put16 = |d: &mut Vec<u8>, word: usize, v: u16| d[word * 2..word * 2 + 2].copy_from_slice(&v.to_le_bytes());
        put16(&mut dest, 3, pixel_width);
        put16(&mut dest, 4, (char_height * num_lines) as u16);
        put16(&mut dest, 5, (pixel_width as usize * char_height * num_lines) as u16);

        let mut lines = Vec::new();
        let mut cur = 0usize;
        let mut prev = HEADER_SIZE;
        let mut ci = 0usize;
        loop {
            let mut dst = prev;
            match align {
                Align::Center => {
                    dst += (pixel_width as usize).saturating_sub(center[ci.min(center.len() - 1)] as usize) >> 1;
                    ci += 1;
                }
                Align::Right => {
                    dst += (pixel_width as usize).saturating_sub(center[ci.min(center.len() - 1)] as usize + 1);
                    ci += 1;
                }
                Align::Left => {}
            }
            let mut line = Vec::new();
            ch = at(&text, cur);
            cur += 1;
            while ch >= 0x20 {
                line.push(ch - 0x20);
                ch = at(&text, cur);
                cur += 1;
            }
            lines.push(line.iter().map(|&c| font_char(c + 0x20)).collect::<String>());
            for &c in &line {
                dst = make_game_character(&cs, c, &mut dest, dst, color, pixel_width as usize);
            }
            prev += line_size;
            if ch < 10 {
                break;
            }
        }

        DisplayedText {
            data: dest,
            width: last_width,
            height: (char_height * num_lines) as u16,
            lines,
            compact_num: 0,
        }
    }

    /// Text::lowTextManager: draw text `nr` into the first free text
    /// compact and return it.
    pub fn low_text_manager(&mut self, nr: u32, width: u16, logic: u16, color: u8, align: Align) -> DisplayedText {
        let text = self.get_text(nr);
        let mut info = self.display_text(text, align, width, color);

        let mut num = FIRST_TEXT_COMPACT;
        let mut c = self.cpt_start(num);
        while self.g(c, f::STATUS) != 0 {
            num += 1;
            c = self.cpt_start(num);
        }
        let item = (num - FIRST_TEXT_COMPACT + FIRST_TEXT_BUFFER) as u16;
        self.s(c, f::FLAG, item);
        self.items[item as usize] = Some(info.data.clone());
        self.text.meta[(item as u32 - FIRST_TEXT_BUFFER) as usize] = Some(TextMeta {
            lines: info.lines.clone(),
            color,
            centered: align == Align::Center,
            line_height: self.charset().height as u16,
        });
        self.s(c, f::LOGIC, logic);
        self.s(c, f::STATUS, (ST_LOGIC | ST_FOREGROUND | ST_RECREATE) as u16);
        self.s(c, f::SCREEN, self.vars[SCREEN as usize] as u16);
        info.compact_num = num as u16;
        info
    }

    /// The item number holding a text compact's sprite.
    pub(crate) fn text_item_of(&self, compact_num: u16) -> usize {
        self.g(self.cpt_start(compact_num as u32), f::FLAG) as usize
    }

    pub(crate) fn fn_pointer_text(&mut self, pointed: u32, mouse_x: u16, _mouse_y: u16) {
        let p = self.cpt_start(pointed);
        let cursor_text = self.g(p, f::CURSOR_TEXT) as u32;
        let t = self.low_text_manager(cursor_text, TEXT_MOUSE_WIDTH as u16, L_CURSOR as u16, 242, Align::Left);
        self.vars[CURSOR_ID as usize] = t.compact_num as u32;
        let left = mouse_x < 150;
        let tw = t.width;
        if self.vars[MENU as usize] != 0 {
            self.text.mouse_ofs_y = TOP_LEFT_Y - 2;
            self.text.mouse_ofs_x =
                if left { TOP_LEFT_X + 24 } else { (TOP_LEFT_X - 8).wrapping_sub(tw) };
        } else {
            self.text.mouse_ofs_y = TOP_LEFT_Y - 10;
            self.text.mouse_ofs_x =
                if left { TOP_LEFT_X + 13 } else { (TOP_LEFT_X - 8).wrapping_sub(tw) };
        }
        let c = self.cpt_start(t.compact_num as u32);
        let (mx, my) = (self.mouse.x, self.mouse.y);
        self.logic_cursor(c, mx, my);
    }

    pub(crate) fn logic_cursor(&mut self, c: usize, mouse_x: u16, mouse_y: u16) {
        let x = (mouse_x as u32).wrapping_add(self.text.mouse_ofs_x) as u16;
        let mut y = (mouse_y as u32).wrapping_add(self.text.mouse_ofs_y) as u16;
        if y < TOP_LEFT_Y as u16 {
            y = TOP_LEFT_Y as u16;
        }
        self.s(c, f::XCOOD, x);
        self.s(c, f::YCOOD, y);
    }

    pub(crate) fn change_text_sprite_color(&mut self, item: usize, color: u8) {
        if let Some(d) = self.items[item].as_mut() {
            let sp = u16::from_le_bytes([d[10], d[11]]) as usize;
            for p in d[HEADER_SIZE..].iter_mut().take(sp) {
                if *p >= 241 {
                    *p = color;
                }
            }
        }
        if let Some(m) = item
            .checked_sub(FIRST_TEXT_BUFFER as usize)
            .and_then(|i| self.text.meta.get_mut(i))
            .and_then(|m| m.as_mut())
        {
            m.color = color;
        }
    }
}

/// Draws one character; returns where the next one starts.
fn make_game_character(cs: &CharSet, c: u8, dest: &mut [u8], at: usize, color: u8, pitch: usize) -> usize {
    let char_width = (*cs.data.get(c as usize).unwrap_or(&0) as u32 + 1).wrapping_sub(cs.spacing) as u8;
    let mut sp = CHAR_SET_HEADER + (cs.height as usize * 4) * c as usize;
    let mut row = at;
    for _ in 0..cs.height {
        let be = |i: usize| u16::from_be_bytes([*cs.data.get(i).unwrap_or(&0), *cs.data.get(i + 1).unwrap_or(&0)]);
        let mut data = be(sp);
        let mut mask = be(sp + 2);
        sp += 4;
        for j in 0..char_width as usize {
            let mask_bit = mask & 0x8000 != 0;
            mask <<= 1;
            let data_bit = data & 0x8000 != 0;
            data <<= 1;
            if mask_bit {
                if let Some(p) = dest.get_mut(row + j) {
                    *p = if data_bit { color } else { 240 }; // 240 is the black edge
                }
            }
        }
        row += pitch;
    }
    at + char_width as usize + cs.spacing as usize * 2 - 1
}

fn replace_all(text: &mut [u8], from: &[u8], to: &[u8]) {
    if let Some(i) = text.windows(from.len()).position(|w| w == from) {
        text[i..i + to.len()].copy_from_slice(to);
    }
}

/// The character a byte of game text stands for, read off the glyphs of
/// the game's main and LINC fonts (files 60150 and 60521, which agree;
/// `typosky glyphs` draws them). The fonts redraw some ASCII codes as
/// accented letters, so German "#ber" is "über". Checked by eye
/// 2026-10-17; 0x84, 0x9B and 0x9C are rare and their readings uncertain.
pub fn font_char(b: u8) -> char {
    // 0x80 to 0x9C; 0x9C looks like n with a circumflex, given as plain n
    const HIGH: [char; 29] = [
        '<', '>', '/', 'Å', 'Ä', 'å', 'É', 'í', 'ó', 'ë', '¿', '¡', 'Ñ', 'ñ', 'ì', 'ò', // 0x80
        'Á', 'à', 'ù', 'Ó', 'æ', 'ã', 'Õ', 'õ', 'Ã', 'Ê', 'Ú', 'ń', 'n', // 0x90
    ];
    match b {
        0x23 => 'ü',
        0x28 => 'ß',
        0x29 => 'ö',
        0x2A => 'ä',
        0x2B => 'é',
        0x2F => 'ç',
        0x3C => 'á',
        0x3D => 'ê',
        0x3E => 'û',
        0x40 => 'ô',
        0x5B => 'â',
        0x5C => 'ú',
        0x5D => 'è',
        0x5E => 'Ç',
        0x5F => 'î',
        0x7B => 'ï',
        0x7C => '´',
        0x7D => 'Ü',
        0x7E => 'Ä',
        0x7F => 'Ö',
        0x20..=0x7F => b as char,
        0x80..=0x9C => HIGH[(b - 0x80) as usize],
        _ => '?',
    }
}
