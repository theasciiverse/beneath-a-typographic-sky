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

/// `typosky headless <cycles> [every] [out]`: runs the game with no screen
/// from the restart point, saving every n-th frame as a PNG.
pub fn headless(dir: &Path, args: &[String]) {
    use crate::engine::host::Headless;
    use crate::engine::sky::Sky;
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
    let mut sky = Sky::new(dir, Box::new(Headless::new())).unwrap_or_else(|e| fail(&e.to_string()));
    sky.start(true);
    for n in 0..cycles {
        for &(c, x, y, b) in &clicks {
            if c == n {
                sky.mouse_moved(x, y);
                sky.mouse.button_pressed(b);
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

pub fn probe(dir: &Path, _args: &[String]) {
    use crate::engine::defs::*;
    use crate::engine::host::Headless;
    use crate::engine::sky::{f, Sky};
    let mut sky = Sky::new(dir, Box::new(Headless::new())).unwrap();
    sky.start(true);
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
        let n: u32 = a.parse().unwrap_or(0);
        println!("{n}: {}", String::from_utf8_lossy(&sky.get_text(n)));
    }
}
