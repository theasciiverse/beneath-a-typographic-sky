//! Turns a true-colour frame into terminal cells.
//!
//! Braille is the native mode: a 2x4 dot cell over 160x48 cells covers the
//! game's 320x192 pixels one dot per pixel, each cell with its own two
//! colours. Half blocks trade resolution for solid colour.

pub type Rgb = [u8; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// U+2800 Braille, 2x4 dots per cell, two colours per cell.
    Braille,
    /// U+2580 upper half block, 1x2 pixels per cell, two colours per cell.
    HalfBlock,
    /// U+2596..U+259F quadrant blocks, 2x2 solid dots per cell, two
    /// colours per cell.
    Quadrant,
}

impl Mode {
    pub fn dots(self) -> (usize, usize) {
        match self {
            Mode::Braille => (2, 4),
            Mode::HalfBlock => (1, 2),
            Mode::Quadrant => (2, 2),
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "braille" => Some(Mode::Braille),
            "half" | "halfblock" => Some(Mode::HalfBlock),
            "quad" | "quadrant" => Some(Mode::Quadrant),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Rgb,
    pub bg: Rgb,
}

/// A source image: `w` x `h` RGB pixels.
pub struct Image<'a> {
    pub w: usize,
    pub h: usize,
    pub px: &'a [Rgb],
}

fn luma(c: Rgb) -> i32 {
    (c[0] as i32 * 299 + c[1] as i32 * 587 + c[2] as i32 * 114) / 1000
}

fn dist(a: Rgb, b: Rgb) -> i32 {
    let d = |i: usize| a[i] as i32 - b[i] as i32;
    // weighted towards green, as the eye is
    2 * d(0) * d(0) + 4 * d(1) * d(1) + 3 * d(2) * d(2)
}

fn mean(cs: &[Rgb]) -> Rgb {
    if cs.is_empty() {
        return [0, 0, 0];
    }
    let mut s = [0u32; 3];
    for c in cs {
        for i in 0..3 {
            s[i] += c[i] as u32;
        }
    }
    let n = cs.len() as u32;
    [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8]
}

/// Samples the source box behind one dot of an output grid of `gw` x `gh`
/// dots (box filter, so any scale works).
fn sample(img: &Image, gw: usize, gh: usize, dx: usize, dy: usize) -> Rgb {
    let x0 = dx * img.w / gw;
    let x1 = ((dx + 1) * img.w / gw).max(x0 + 1).min(img.w);
    let y0 = dy * img.h / gh;
    let y1 = ((dy + 1) * img.h / gh).max(y0 + 1).min(img.h);
    let mut s = [0u32; 3];
    let mut n = 0u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let c = img.px[y * img.w + x];
            for i in 0..3 {
                s[i] += c[i] as u32;
            }
            n += 1;
        }
    }
    [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8]
}

/// Splits up to 8 colours into two clusters. Returns (light, dark, mask of
/// members in the light cluster).
fn split2(cs: &[Rgb]) -> (Rgb, Rgb, u8) {
    // seed with the lightest and darkest
    let (mut a, mut b) = (cs[0], cs[0]);
    for &c in cs {
        if luma(c) > luma(a) {
            a = c;
        }
        if luma(c) < luma(b) {
            b = c;
        }
    }
    let mut mask = 0u8;
    for _ in 0..3 {
        mask = 0;
        let (mut la, mut lb) = (Vec::with_capacity(8), Vec::with_capacity(8));
        for (i, &c) in cs.iter().enumerate() {
            if dist(c, a) <= dist(c, b) {
                mask |= 1 << i;
                la.push(c);
            } else {
                lb.push(c);
            }
        }
        if la.is_empty() || lb.is_empty() {
            break;
        }
        a = mean(&la);
        b = mean(&lb);
    }
    (a, b, mask)
}

/// Braille dot bit for dot (x, y) within a 2x4 cell.
const BRAILLE_BIT: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// Renders `img` into `cols` x `rows` cells.
pub fn render(img: &Image, mode: Mode, cols: usize, rows: usize) -> Vec<Cell> {
    let (cw, ch) = mode.dots();
    let (gw, gh) = (cols * cw, rows * ch);
    let mut samples: Vec<[Rgb; 8]> = Vec::with_capacity(cols * rows);
    for cy in 0..rows {
        for cx in 0..cols {
            let mut dots = [[0u8; 3]; 8];
            for y in 0..ch {
                for x in 0..cw {
                    dots[y * cw + x] = sample(img, gw, gh, cx * cw + x, cy * ch + y);
                }
            }
            samples.push(dots);
        }
    }
    // Braille: the brightness around each cell, to decide which of its two
    // colours is the background (see braille_cell)
    let around: Vec<i32> = if mode == Mode::Braille {
        let n = cw * ch;
        let cell_luma: Vec<i32> = samples.iter().map(|d| d[..n].iter().map(|&c| luma(c)).sum::<i32>() / n as i32).collect();
        (0..rows * cols)
            .map(|i| {
                let (cx, cy) = ((i % cols) as i32, (i / cols) as i32);
                let (mut sum, mut cnt) = (0, 0);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (x, y) = (cx + dx, cy + dy);
                        if x >= 0 && y >= 0 && (x as usize) < cols && (y as usize) < rows {
                            sum += cell_luma[y as usize * cols + x as usize];
                            cnt += 1;
                        }
                    }
                }
                sum / cnt
            })
            .collect()
    } else {
        Vec::new()
    };
    samples
        .iter()
        .enumerate()
        .map(|(i, dots)| match mode {
            Mode::HalfBlock => Cell { ch: '▀', fg: dots[0], bg: dots[1] },
            Mode::Braille => braille_cell(dots, around[i]),
            Mode::Quadrant => quadrant_cell(dots),
        })
        .collect()
}

/// `around` is the mean brightness of the cell and its eight neighbours.
fn braille_cell(dots: &[Rgb; 8], around: i32) -> Cell {
    let (light, dark, mask) = split2(dots);
    if mask == 0xFF || mask == 0 || dist(light, dark) < 60 {
        // flat: a space shows the background only
        return Cell { ch: ' ', fg: light, bg: mean(dots) };
    }
    // The dots are drawn in fg, the rest of the cell in bg. The colour
    // nearer the surroundings' brightness is the background, so a bright
    // stroke on a dark screen stays dots even where it covers most of a
    // cell; deciding by count alone lit such cells up as solid blobs.
    let lit_light = (luma(light) - around).abs() > (luma(dark) - around).abs();
    let mut bits = 0u8;
    for y in 0..4 {
        for x in 0..2 {
            let i = y * 2 + x;
            let in_light = mask & (1 << i) != 0;
            if in_light == lit_light {
                bits |= BRAILLE_BIT[x][y];
            }
        }
    }
    let (fg, bg) = if lit_light { (light, dark) } else { (dark, light) };
    Cell { ch: char::from_u32(0x2800 + bits as u32).unwrap(), fg, bg }
}

/// Quadrant characters by bit pattern: 1 top left, 2 top right,
/// 4 bottom left, 8 bottom right.
const QUADRANTS: [char; 16] =
    [' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█'];

fn quadrant_cell(dots: &[Rgb; 8]) -> Cell {
    let (light, dark, mask) = split2(&dots[..4]);
    if mask & 0xF == 0xF || mask & 0xF == 0 {
        return Cell { ch: ' ', fg: light, bg: mean(&dots[..4]) };
    }
    Cell { ch: QUADRANTS[(mask & 0xF) as usize], fg: light, bg: dark }
}

/// Writes cells as ANSI truecolour text.
pub fn to_ansi(cells: &[Cell], cols: usize) -> String {
    let mut s = String::new();
    for row in cells.chunks(cols) {
        let mut last: Option<(Rgb, Rgb)> = None;
        for c in row {
            if last != Some((c.fg, c.bg)) {
                s += &format!(
                    "\x1b[38;2;{};{};{};48;2;{};{};{}m",
                    c.fg[0], c.fg[1], c.fg[2], c.bg[0], c.bg[1], c.bg[2]
                );
                last = Some((c.fg, c.bg));
            }
            s.push(c.ch);
        }
        s += "\x1b[0m\n";
    }
    s
}

/// Simulates how the cells look in a terminal with `cw` x `ch` pixel cells,
/// for previews and tests. Braille dots are drawn as round-ish blobs.
pub fn simulate(cells: &[Cell], cols: usize, cw: usize, chh: usize) -> (usize, usize, Vec<Rgb>) {
    let rows = cells.len() / cols;
    let (w, h) = (cols * cw, rows * chh);
    let mut px = vec![[0u8; 3]; w * h];
    for (i, c) in cells.iter().enumerate() {
        let (ox, oy) = ((i % cols) * cw, (i / cols) * chh);
        for y in 0..chh {
            for x in 0..cw {
                let on = match c.ch {
                    '▀' => y < chh / 2,
                    q if QUADRANTS.contains(&q) => {
                        let bits = QUADRANTS.iter().position(|&c| c == q).unwrap();
                        let bit = (if x * 2 >= cw { 2 } else { 1 }) * (if y * 2 >= chh { 4 } else { 1 });
                        bits & bit != 0
                    }
                    ' ' => false,
                    b if ('\u{2800}'..='\u{28FF}').contains(&b) => {
                        let bits = b as u32 - 0x2800;
                        let (dx, dy) = (x * 2 / cw, y * 4 / chh);
                        let bit = BRAILLE_BIT[dx][dy] as u32;
                        // dot is a centred square about 60% of its slot
                        let sx = (x * 2 % cw) as f32 / cw as f32;
                        let sy = (y * 4 % chh) as f32 / chh as f32;
                        bits & bit != 0 && (0.2..0.8).contains(&sx) && (0.2..0.8).contains(&sy)
                    }
                    _ => false,
                };
                px[(oy + y) * w + ox + x] = if on { c.fg } else { c.bg };
            }
        }
    }
    (w, h, px)
}
