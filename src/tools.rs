//! Data exploration: `typosky dump ...`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::engine::cpt::Cpt;
use crate::engine::disk::{Disk, Header};

pub fn dump(dir: &Path, args: &[String]) {
    let disk = Disk::open(dir).unwrap_or_else(|e| fail(&format!("{}: {e}", dir.display())));
    let cpt = Cpt::open(dir, disk.game_version).unwrap_or_else(|e| fail(&format!("sky.cpt: {e}")));
    let out = PathBuf::from(args.get(1).map(String::as_str).unwrap_or("out"));
    match args.first().map(String::as_str) {
        Some("files") => {
            println!("version {}", disk.game_version);
            for n in disk.file_numbers() {
                match disk.load(n) {
                    Some(d) => {
                        let h = if d.len() >= 22 { Header::parse(&d) } else { Header::default() };
                        println!(
                            "{n:5} {:>7} bytes  flag {:04x} {}x{} n {} off {},{}",
                            d.len(),
                            h.flag,
                            h.width,
                            h.height,
                            h.n_sprites,
                            h.offset_x,
                            h.offset_y
                        );
                    }
                    None => println!("{n:5} unreadable"),
                }
            }
        }
        Some("cpts") => {
            for l in 0..cpt.num_lists() {
                for i in 0..cpt.list_len(l) {
                    let id = ((l << 12) | i) as u16;
                    if let Some(e) = cpt.entry(id) {
                        println!("{id:04x} type {} words {:5} {}", e.kind, e.size, cpt.name(id));
                    }
                }
            }
        }
        Some("screens") => {
            fs::create_dir_all(&out).ok();
            let pal_id = args.get(2).and_then(|s| cpt.find(s));
            let mut n_written = 0;
            for n in disk.file_numbers() {
                let Some(d) = disk.load(n) else { continue };
                if d.len() != 320 * 192 {
                    continue;
                }
                let pal = match pal_id {
                    Some(id) => cpt.bytes(id),
                    None => (0..768).map(|i| (i / 3 / 4) as u8).collect(),
                };
                let pixels = untile(&d);
                write_png(&out.join(format!("screen-{n}.png")), 320, 192, &pixels, &pal);
                n_written += 1;
            }
            println!("{n_written} screens written to {}", out.display());
        }
        Some("rooms") => {
            fs::create_dir_all(&out).ok();
            for (room, file, pal) in rooms(&disk, &cpt) {
                let d = disk.load(file).expect("listed screen loads");
                let path = out.join(format!("room-{room:02}.png"));
                write_png(&path, 320, 192, &untile(&d), &cpt.bytes(pal));
                println!("room {room:3}: file {file:5}, palette {}", cpt.name(pal));
            }
        }
        _ => fail("dump what? files, cpts, screens or rooms"),
    }
}

/// (room number, layer-0 file, palette compact) for every room whose fast
/// list and palette can be found by name.
pub fn rooms(disk: &Disk, cpt: &Cpt) -> Vec<(u32, u16, u16)> {
    let mut found = Vec::new();
    for l in 0..cpt.num_lists() {
        for i in 0..cpt.list_len(l) {
            let id = ((l << 12) | i) as u16;
            let name = cpt.name(id).to_ascii_lowercase();
            if !name.contains("fast_list") {
                continue;
            }
            let Some(room) = name
                .split(|c: char| !c.is_ascii_digit())
                .find(|s| !s.is_empty())
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let e = cpt.entry(id).unwrap();
            let screen = cpt.raw[e.start..e.start + e.size as usize]
                .iter()
                .map(|w| w & 0x7FFF)
                .find(|&f| f != 0 && disk.load(f).is_some_and(|d| d.len() == 320 * 192));
            let pal = [format!("s{room}_pal"), format!("sc{room}_palette")]
                .iter()
                .find_map(|n| cpt.find(n));
            if let (Some(file), Some(pal)) = (screen, pal) {
                if !found.iter().any(|&(r, _, _)| r == room) {
                    found.push((room, file, pal));
                }
            }
        }
    }
    found.sort();
    found
}

/// Layer-0 screens are stored as 16x8 blocks, 20 across and 24 down.
pub fn untile(d: &[u8]) -> Vec<u8> {
    let mut px = vec![0u8; 320 * 192];
    let mut src = 0;
    for by in 0..24 {
        for bx in 0..20 {
            for y in 0..8 {
                let row = (by * 8 + y) * 320 + bx * 16;
                px[row..row + 16].copy_from_slice(&d[src..src + 16]);
                src += 16;
            }
        }
    }
    px
}

/// Writes 8-bit indexed pixels with a VGA (0..63) palette as an RGB PNG.
pub fn write_png(path: &Path, w: u32, h: u32, pixels: &[u8], vga_pal: &[u8]) {
    let conv = |v: u8| (v << 2) | (v >> 4);
    let mut rgb = Vec::with_capacity(pixels.len() * 3);
    for &p in pixels {
        let i = p as usize * 3;
        for c in 0..3 {
            rgb.push(conv(*vga_pal.get(i + c).unwrap_or(&0) & 63));
        }
    }
    let f = fs::File::create(path).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())));
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut wr| wr.write_image_data(&rgb))
        .unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())));
}

fn fail(msg: &str) -> ! {
    eprintln!("typosky: {msg}");
    std::process::exit(1);
}

/// `typosky look <room> <mode> <cols> <rows> [out]`: renders one room
/// background as terminal cells, writes the ANSI text and a simulated PNG.
pub fn look(dir: &Path, args: &[String]) {
    use crate::term::raster::{self, Image, Mode};
    let disk = Disk::open(dir).unwrap_or_else(|e| fail(&format!("{}: {e}", dir.display())));
    let cpt = Cpt::open(dir, disk.game_version).unwrap_or_else(|e| fail(&format!("sky.cpt: {e}")));
    let arg = |i: usize| args.get(i).map(String::as_str);
    let room: u32 = arg(0).and_then(|s| s.parse().ok()).unwrap_or_else(|| fail("room?"));
    let mode = arg(1).and_then(Mode::parse).unwrap_or(Mode::Braille);
    let cols: usize = arg(2).and_then(|s| s.parse().ok()).unwrap_or(160);
    let rows: usize = arg(3).and_then(|s| s.parse().ok()).unwrap_or(48);
    let out = PathBuf::from(arg(4).unwrap_or("out/look"));
    let (_, file, pal) = rooms(&disk, &cpt)
        .into_iter()
        .find(|r| r.0 == room)
        .unwrap_or_else(|| fail(&format!("room {room} not found")));
    let pal = cpt.bytes(pal);
    let conv = |v: u8| ((v & 63) << 2) | ((v & 63) >> 4);
    let px: Vec<[u8; 3]> = untile(&disk.load(file).unwrap())
        .iter()
        .map(|&p| {
            let i = p as usize * 3;
            [conv(pal[i]), conv(pal[i + 1]), conv(pal[i + 2])]
        })
        .collect();
    let cells = raster::render(&Image { w: 320, h: 192, px: &px }, mode, cols, rows);
    fs::create_dir_all(&out).ok();
    let name = format!("room-{room:02}-{mode:?}-{cols}x{rows}").to_lowercase();
    fs::write(out.join(format!("{name}.ans")), raster::to_ansi(&cells, cols)).ok();
    let (w, h, sim) = raster::simulate(&cells, cols, 8, 16);
    let flat: Vec<u8> = sim.iter().flatten().copied().collect();
    let f = fs::File::create(out.join(format!("{name}.png"))).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&flat).unwrap();
    println!("{}/{name}.ans and .png", out.display());
}

/// A headless host that saves the screen as a PNG every `every_ms` of its
/// clock and can press Escape at a given time: for watching the intro,
/// which runs inside the engine without returning to the tool.
struct IntroRecorder {
    inner: crate::engine::host::Headless,
    out: PathBuf,
    every_ms: u64,
    next_shot: u64,
    skip_at: Option<u64>,
    last_texts: Vec<String>,
}

impl crate::engine::host::Host for IntroRecorder {
    fn millis(&self) -> u64 {
        self.inner.millis()
    }
    fn sleep(&mut self, ms: u32) {
        self.inner.sleep(ms)
    }
    fn poll(&mut self) -> Vec<crate::engine::host::HostEvent> {
        use crate::engine::host::{Action, HostEvent};
        if self.skip_at.is_some_and(|t| self.inner.now >= t) {
            self.skip_at = None;
            println!("{:7} ms: Escape", self.inner.now);
            self.inner.events.push_back(HostEvent::Action(Action::Skip));
        }
        self.inner.poll()
    }
    fn present(&mut self, frame: &crate::engine::host::Frame) {
        self.inner.present(frame);
        let now = self.inner.now;
        let texts: Vec<String> = frame.texts.iter().map(|t| format!("{} at {},{}", t.lines.join(" / "), t.x, t.y)).collect();
        if texts != self.last_texts {
            println!("{now:7} ms: texts {texts:?}");
            self.last_texts = texts;
        }
        if now >= self.next_shot {
            self.next_shot = now - now % self.every_ms + self.every_ms;
            let pal: Vec<u8> = frame.palette.iter().map(|&v| v >> 2).collect();
            write_png(&self.out.join(format!("intro-{now:07}.png")), 320, 200, frame.pixels, &pal);
        }
    }
}

/// `typosky headless <cycles> [every] [out]`: runs the game with no screen
/// from the restart point, saving every n-th frame as a PNG.
///
/// With `--intro` it plays the CD intro first (`--floppy` for the floppy
/// one), saving the screen every `--every-ms=N` milliseconds of game time
/// (default 1000) as intro-<ms>.png; `--skip-at=N` presses Escape at N ms.
/// The game then runs from where the intro leaves it.
pub fn headless(dir: &Path, args: &[String]) {
    use crate::engine::host::Headless;
    use crate::engine::sky::Sky;
    let flag = |name: &str| args.iter().any(|a| a == name);
    let flag_val = |name: &str| -> Option<u64> {
        args.iter().find_map(|a| a.strip_prefix(name)?.strip_prefix('=')?.parse().ok())
    };
    let intro = flag("--intro") || flag("--floppy");
    let floppy = flag("--floppy");
    let every_ms = flag_val("--every-ms").unwrap_or(1000).max(1);
    let skip_at = flag_val("--skip-at");
    let args: Vec<String> = args.iter().filter(|a| !a.starts_with("--")).cloned().collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    let cycles: u32 = arg(0).and_then(|s| s.parse().ok()).unwrap_or(100);
    let every: u32 = arg(1).and_then(|s| s.parse().ok()).unwrap_or(25);
    let out = PathBuf::from(arg(2).unwrap_or("out/headless"));
    fs::create_dir_all(&out).ok();
    // further args: "cycle:x,y[,button]" clicks (button 2 left, 1 right)
    let clicks: Vec<(u32, u16, u16, u8)> = args
        .iter()
        .skip(3)
        .filter_map(|a| {
            let (n, rest) = a.split_once(':')?;
            let v: Vec<u16> = rest.split(',').filter_map(|x| x.parse().ok()).collect();
            Some((n.parse().ok()?, *v.first()?, *v.get(1)?, *v.get(2).unwrap_or(&2) as u8))
        })
        .collect();
    let mut sky = if intro {
        let host = IntroRecorder {
            inner: Headless::new(),
            out: out.clone(),
            every_ms,
            next_shot: 0,
            skip_at,
            last_texts: Vec::new(),
        };
        Sky::new(dir, Box::new(host)).unwrap_or_else(|e| fail(&e.to_string()))
    } else {
        Sky::new(dir, Box::new(Headless::new())).unwrap_or_else(|e| fail(&e.to_string()))
    };
    if !intro {
        sky.start(true);
    } else if !floppy {
        let t0 = sky.host.millis();
        sky.start(false);
        println!("intro over after {} ms (past_intro {})", sky.host.millis() - t0, sky.sys.past_intro);
    } else {
        // Sky::start with the floppy intro, as ScummVM's go() with alt_intro
        let t0 = sky.host.millis();
        let played = sky.do_intro_alt(true);
        println!("floppy intro {} after {} ms", if played { "played" } else { "skipped" }, sky.host.millis() - t0);
        if !sky.quit {
            sky.clear_screen();
            if played {
                sky.init_screen0();
            } else {
                sky.restart_game();
            }
        }
    }
    for n in 0..cycles {
        for &(c, x, y, b) in &clicks {
            if c == n {
                sky.mouse_moved(x, y);
                if b != 0 {
                    sky.mouse.button_pressed(b);
                }
                println!("cycle {n}: click {b} at {x},{y}");
            }
            if c + 3 == n || c + 1 == n {
                sky.mouse_moved(x, y);
            }
        }
        sky.tick();
        sky.delay(80);
        if n % every == 0 || n + 1 == cycles {
            let pal: Vec<u8> = sky.screen.sys_pal.iter().map(|&v| v >> 2).collect();
            write_png(&out.join(format!("frame-{n:05}.png")), 320, 200, &sky.screen.display, &pal);
            let texts: Vec<String> = sky.screen.texts_shown.iter().map(|t| t.lines.join(" / ")).collect();
            println!("cycle {n}: screen {} texts {:?}", sky.vars[crate::engine::defs::SCREEN as usize], texts);
        }
    }
}

pub fn probe(dir: &Path, args: &[String]) {
    use crate::engine::defs::*;
    use crate::engine::host::Headless;
    use crate::engine::sky::{f, Sky};
    let mut sky = Sky::new(dir, Box::new(Headless::new())).unwrap();
    sky.start(true);
    // "die": walk down to the guard first, to probe the death screen
    if args.first().map(String::as_str) == Some("die") {
        for n in 0..600 {
            if n == 10 || n == 11 || n == 13 {
                sky.mouse_moved(60, 150);
            }
            if n == 10 {
                sky.mouse.button_pressed(2);
            }
            sky.tick();
            sky.delay(80);
            if sky.vars[SCREEN as usize] == 102 {
                break;
            }
        }
    }
    macro_rules! v { ($n:expr) => { sky.vars[$n as usize] }; }
    println!("SCREEN {} LOGIC_LIST {:#x} SECTION {} LAYER0 {} pal {:#x} mouse_status {}", v!(SCREEN), v!(LOGIC_LIST_NO), v!(CUR_SECTION), v!(LAYER_0_ID), sky.sys.current_palette, v!(MOUSE_STATUS));
    for _ in 0..30 { sky.tick(); }
    let mut ml = v!(MOUSE_LIST_NO);
    println!("mouse list {:#x} status {}", ml, sky.vars[MOUSE_STATUS as usize]);
    'outer: loop {
        let mut p = sky.cpt_start(ml);
        loop {
            let id = sky.w(p); p += 1;
            if id == 0 { break 'outer; }
            if id == 0xFFFF { ml = sky.w(p) as u32; continue 'outer; }
            let c = sky.cpt_start(id as u32);
            println!("  M {id:#06x} {:20} status {:#x} screen {} at {},{} rel {},{} size {}x{} text {}", sky.cpt.name(id), sky.g(c,f::STATUS), sky.g(c,f::SCREEN), sky.g(c,f::XCOOD), sky.g(c,f::YCOOD), sky.g(c,f::MOUSE_REL_X) as i16, sky.g(c,f::MOUSE_REL_Y) as i16, sky.g(c,f::MOUSE_SIZE_X), sky.g(c,f::MOUSE_SIZE_Y), sky.g(c,f::CURSOR_TEXT));
        }
    }
    let mut p = sky.cpt_start(v!(LOGIC_LIST_NO));
    loop {
        let id = sky.w(p); p += 1;
        if id == 0 { break; }
        if id == 0xFFFF { p = sky.cpt_start(sky.w(p) as u32); continue; }
        let c = sky.cpt_start(id as u32);
        println!("  {id:#06x} {:20} logic {} status {:#x} screen {} mode {} base {:#x}:{}", sky.cpt.name(id), sky.g(c,f::LOGIC), sky.g(c,f::STATUS), sky.g(c,f::SCREEN), sky.g(c,f::MODE), sky.g(c,f::BASE_SUB), sky.g(c,f::BASE_SUB_OFF));
    }
}

pub fn text(dir: &Path, args: &[String]) {
    use crate::engine::host::Headless;
    use crate::engine::sky::Sky;
    let mut sky = Sky::new(dir, Box::new(Headless::new())).unwrap();
    for a in args {
        if let Some(l) = a.strip_prefix("--lang=") {
            sky.set_language(l.parse().unwrap_or(0));
            continue;
        }
        let n: u32 = a.parse().unwrap_or(0);
        let t: String = sky.get_text(n).iter().map(|&b| crate::engine::text::font_char(b)).collect();
        println!("{n}: {t}");
    }
}

/// `typosky monkey <cycles> <seed>`: random pointer moves and clicks,
/// headless, to shake out crashes. Prints the rooms visited.
pub fn monkey(dir: &Path, args: &[String]) {
    use crate::engine::defs::SCREEN;
    use crate::engine::host::Headless;
    use crate::engine::sky::Sky;
    let cycles: u32 = args.first().and_then(|s| s.parse().ok()).unwrap_or(5000);
    let mut seed: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut sky = Sky::new(dir, Box::new(Headless::new())).unwrap();
    sky.start(true);
    // the scripted opening first, so the monkey starts in room 1
    let opening = [(5u32, 23u16, 70u16, 1u8), (150, 160, 1, 0), (160, 40, 10, 1), (175, 280, 60, 0), (180, 280, 60, 1)];
    for n in 0..600 {
        for &(c, x, y, b) in &opening {
            if c == n || c + 1 == n || c + 3 == n {
                sky.mouse_moved(x, y);
            }
            if c == n && b != 0 {
                sky.mouse.button_pressed(b);
            }
        }
        sky.tick();
        sky.delay(80);
    }
    let start_save = sky.prepare_save_data();
    let mut rooms = std::collections::BTreeSet::new();
    let mut last_room = u32::MAX;
    for n in 0..cycles {
        let r = rnd();
        if sky.vars[SCREEN as usize] == 102 {
            // the death screen: back to room 1 rather than click Quit
            sky.parse_save_data(&start_save).unwrap();
        }
        if r % 3 == 0 {
            sky.mouse_moved((r >> 8) as u16 % 320, (r >> 20) as u16 % 200);
        }
        if r % 17 == 0 {
            sky.mouse.button_pressed(if (r >> 30) % 4 == 0 { 1 } else { 2 });
        }
        sky.tick();
        sky.delay(80);
        let room = sky.vars[SCREEN as usize];
        if room != last_room {
            log::info!("cycle {n}: room {room}");
            last_room = room;
        }
        rooms.insert(room);
        if sky.quit {
            println!("quit at cycle {n}");
            break;
        }
    }
    println!("rooms visited: {rooms:?}");
}

/// `typosky modes <frame.png> <cols> <rows> <y0> <y1> <out.png>`: renders
/// pixel rows y0..y1 of a 320x200 frame in each terminal mode, stacked,
/// as the terminal would show them, to compare.
pub fn modes(args: &[String]) {
    use crate::term::raster::{self, Image, Mode};
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_else(|| fail("modes <png> <cols> <rows> <y0> <y1> <out>"));
    let dec = png::Decoder::new(std::io::BufReader::new(fs::File::open(arg(0)).unwrap()));
    let mut rd = dec.read_info().unwrap();
    let mut buf = vec![0; rd.output_buffer_size()];
    let info = rd.next_frame(&mut buf).unwrap();
    let ch = info.color_type.samples();
    let px: Vec<[u8; 3]> = buf[..info.buffer_size()].chunks(ch).map(|p| [p[0], p[1], p[2]]).collect();
    let (cols, rows): (usize, usize) = (arg(1).parse().unwrap(), arg(2).parse().unwrap());
    let (y0, y1): (usize, usize) = (arg(3).parse().unwrap(), arg(4).parse().unwrap());
    let (r0, r1) = (y0 * rows / 200, (y1 * rows + 199) / 200);
    let mut out: Vec<[u8; 3]> = Vec::new();
    let mut w = 0;
    for mode in [Mode::Braille, Mode::HalfBlock, Mode::Quadrant] {
        let cells = raster::render(&Image { w: 320, h: 200, px: &px }, mode, cols, rows);
        let band = &cells[r0 * cols..r1 * cols];
        let (sw, _sh, sim) = raster::simulate(band, cols, 8, 16);
        w = sw;
        out.extend(sim);
        out.extend(std::iter::repeat([40, 40, 40]).take(sw * 4));
    }
    let h = out.len() / w;
    let flat: Vec<u8> = out.iter().flatten().copied().collect();
    let f = fs::File::create(arg(5)).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&flat).unwrap();
}

/// `typosky glyphs <out.png>`: the three game fonts' glyphs for bytes
/// 0x20..0x9F, one font per row band, 16 glyphs per line, zoomed 4x.
pub fn glyphs(dir: &Path, args: &[String]) {
    let disk = Disk::open(dir).unwrap_or_else(|e| fail(&e.to_string()));
    let fonts = [(60150u16, 12usize), (60521, 12), (60520, 12)];
    let (gw, gh, z) = (16usize, 14usize, 4usize);
    let lines = 8; // 128 glyphs / 16
    let (w, h) = (16 * gw * z, fonts.len() * lines * gh * z);
    let mut px = vec![[20u8, 20, 30]; w * h];
    for (fi, &(file, height)) in fonts.iter().enumerate() {
        let d = disk.load(file).unwrap();
        for c in 0..128usize {
            let base = 128 + height * 4 * c;
            let (gx, gy) = (c % 16, fi * lines + c / 16);
            for row in 0..height {
                let be = |i: usize| u16::from_be_bytes([*d.get(i).unwrap_or(&0), *d.get(i + 1).unwrap_or(&0)]);
                let (data, mask) = (be(base + row * 4), be(base + row * 4 + 2));
                for col in 0..16 {
                    let bit = 0x8000 >> col;
                    let v = if mask & bit != 0 { if data & bit != 0 { [255, 255, 255] } else { [90, 60, 60] } } else { continue };
                    for zy in 0..z {
                        for zx in 0..z {
                            let x = (gx * gw + col) * z + zx;
                            let y = (gy * gh + row) * z + zy;
                            px[y * w + x] = v;
                        }
                    }
                }
            }
        }
    }
    let flat: Vec<u8> = px.iter().flatten().copied().collect();
    let f = fs::File::create(args.first().map(String::as_str).unwrap_or("glyphs.png")).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&flat).unwrap();
}

/// `typosky trace <save> <x> <y> <button> <cycles>`: restores a save,
/// clicks once and prints Foster's logic every cycle.
pub fn trace(dir: &Path, args: &[String]) {
    use crate::engine::defs::*;
    use crate::engine::host::Headless;
    use crate::engine::sky::{f, Sky};
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or_else(|| fail("trace <save> <x> <y> <button> <cycles>"));
    let mut sky = Sky::new(dir, Box::new(Headless::new())).unwrap();
    sky.start(true);
    let data = fs::read(arg(0)).unwrap();
    sky.restore_data(&data).unwrap();
    let (x, y, b): (u16, u16, u8) = (arg(1).parse().unwrap(), arg(2).parse().unwrap(), arg(3).parse().unwrap());
    let cycles: u32 = arg(4).parse().unwrap();
    // optional sixth argument: the cycle to click at (default 3)
    let at: u32 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(3);
    for n in 0..cycles {
        if n + 3 >= at && n <= at {
            sky.mouse_moved(x, y);
        }
        if n == at {
            sky.mouse.button_pressed(b);
        }
        sky.tick();
        sky.delay(80);
        let c = sky.cpt_start(ID_FOSTER);
        let g = |fld| sky.g(c, fld);
        println!(
            "{n:4} room {:3} logic {:2} mode {:2} at {},{} dir {} sub {:#06x}:{} {:#06x}:{} {:#06x}:{} waiting {} ar {},{} down {} gp {:#06x}:{}",
            sky.vars[SCREEN as usize], g(f::LOGIC), g(f::MODE), g(f::XCOOD), g(f::YCOOD), g(f::DIR),
            g(f::BASE_SUB), g(f::BASE_SUB_OFF), g(f::ACTION_SUB), g(f::ACTION_SUB_OFF), g(f::GET_TO_SUB), g(f::GET_TO_SUB_OFF),
            g(f::WAITING_FOR), g(f::AR_TARGET_X), g(f::AR_TARGET_Y), g(f::DOWN_FLAG), g(f::GRAFIX_PROG_ID), g(f::GRAFIX_PROG_POS)
        );
        let l = sky.cpt_start(ID_LAMB);
        let ms = sky.mega_set(l);
        println!(
            "     lamb room {} logic {} at {},{} status {:#x} mood {} dir {} megaset {} col_off {} col_w {} last {}",
            sky.g(l, f::SCREEN), sky.g(l, f::LOGIC), sky.g(l, f::XCOOD), sky.g(l, f::YCOOD), sky.g(l, f::STATUS),
            sky.g(l, f::MOOD), sky.g(l, f::DIR), sky.g(l, f::MEGA_SET), sky.w(ms + 1), sky.w(ms + 2), sky.w(ms + 3)
        );
    }
}
