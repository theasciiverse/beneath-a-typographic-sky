//! Engine tests against the real game data. They skip when the data is
//! not there (set TYPOSKY_DATA, or keep reference/bass-cd-1.2).

use std::path::PathBuf;

use super::defs::SCREEN;
use super::host::Headless;
use super::sky::Sky;

fn data() -> Option<PathBuf> {
    let d = std::env::var_os("TYPOSKY_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("reference/bass-cd-1.2"));
    d.join("sky.dsk").exists().then_some(d)
}

fn new_game() -> Option<Sky> {
    let mut sky = Sky::new(&data()?, Box::new(Headless::new())).expect("engine starts");
    sky.start(true);
    Some(sky)
}

/// Runs cycles, applying (cycle, x, y, button) inputs; button 0 only moves.
fn play(sky: &mut Sky, cycles: u32, input: &[(u32, u16, u16, u8)]) {
    for n in 0..cycles {
        for &(c, x, y, b) in input {
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
}

/// The first puzzle: take the rung, pry the door, escape to room 1.
const OPENING: [(u32, u16, u16, u8); 5] =
    [(5, 23, 70, 1), (150, 160, 1, 0), (160, 40, 10, 1), (175, 280, 60, 0), (180, 280, 60, 1)];

#[test]
fn opening_puzzle_leads_to_room_1() {
    let Some(mut sky) = new_game() else { return };
    assert_eq!(sky.vars[SCREEN as usize], 0);
    play(&mut sky, 600, &OPENING);
    assert_eq!(sky.vars[SCREEN as usize], 1, "Foster should have escaped through the door");
}

#[test]
fn save_restore_round_trip() {
    let Some(mut sky) = new_game() else { return };
    play(&mut sky, 300, &OPENING);
    let saved = sky.prepare_save_data();
    let mut other = new_game().unwrap();
    other.parse_save_data(&saved).expect("restores");
    // a restore switches all text compacts off, as ScummVM's does; any
    // other difference is a bug
    let again = other.prepare_save_data();
    assert_eq!(again.len(), saved.len());
    let mut at = 4 * 9 - 4 + 838 * 4 + 60 * 4;
    for &id in &sky.cpt.save_ids {
        let n = sky.cpt.entry(id).unwrap().size as usize * 2;
        if !(super::defs::CPT_TEXT_1..=super::defs::CPT_TEXT_11).contains(&(id as u32)) {
            assert_eq!(saved[at..at + n], again[at..at + n], "compact {} differs", sky.cpt.name(id));
        }
        at += n;
    }
    assert_eq!(saved[..3624], again[..3624]);
    // and both go on the same way
    play(&mut sky, 300, &[]);
    play(&mut other, 300, &[]);
    assert_eq!(sky.vars[SCREEN as usize], other.vars[SCREEN as usize]);
}

#[test]
fn texts_decode() {
    let Some(mut sky) = new_game() else { return };
    assert_eq!(sky.get_text(4148), b"Door");
    assert_eq!(sky.get_text(4097), b"What YOU want?");
}

/// The dialogue chooser: two lines set up as fnAskThis does, then
/// fnChooser; both appear as text lines in the right places.
#[test]
fn chooser_shows_lines() {
    let Some(mut sky) = new_game() else { return };
    play(&mut sky, 20, &[]);
    sky.mcode(69, 0, 0, 0); // fnNewList
    sky.mcode(70, 4097, 0, 0); // fnAskThis
    sky.mcode(70, 4148, 0, 0);
    let foster = sky.cpt_start(super::defs::ID_FOSTER);
    sky.compact = foster;
    assert!(!sky.mcode(39, 0, 0, 0), "fnChooser drops out of the script");
    sky.recreate();
    sky.sprite_engine();
    sky.flip(false);
    let shown: Vec<String> = sky.screen.texts_shown.iter().map(|t| t.lines.join(" ")).collect();
    assert!(shown.contains(&"What YOU want?".to_string()), "{shown:?}");
    assert!(shown.contains(&"Door".to_string()), "{shown:?}");
    // the second line sits one font height below the first, at the top
    let door = sky.screen.texts_shown.iter().find(|t| t.lines == ["Door"]).unwrap();
    assert_eq!((door.x, door.y), (0, 12));
}

/// Text numbers past the end of their section once made get_text read
/// zeros forever, growing by about 250 MB a second. They must end.
#[test]
fn out_of_range_texts_end() {
    let Some(mut sky) = new_game() else { return };
    for nr in [0x0FFF, 0x1FFF, 0x7FFF, 30000, 65535] {
        let t = sky.get_text(nr);
        assert!(t.len() < 1024, "text {nr} ran on for {} bytes", t.len());
    }
}

#[test]
fn font_characters() {
    use super::text::font_char;
    let s: String = b"#ber<<UNKNOWN>>".iter().map(|&b| font_char(b)).collect();
    assert_eq!(s, "überááUNKNOWNûû");
    let s: String = [0x80u8, 0x80, b'O', b'K', 0x81, 0x81].iter().map(|&b| font_char(b)).collect();
    assert_eq!(s, "<<OK>>");
}

/// On a coarse terminal a cell covers several pixels; a pointer cell that
/// covers part of a thin hotspot snaps onto it.
#[test]
fn pointer_snaps_to_thin_hotspots() {
    let Some(dir) = data() else { return };
    let mut host = Headless::new();
    host.reach = (2, 4);
    let mut sky = Sky::new(&dir, Box::new(host)).expect("engine starts");
    sky.start(true);
    play(&mut sky, 20, &[]);
    // the rung's hotspot is screen x 15..32, y 69..72, three pixels tall
    assert_eq!(sky.snap_pointer(20, 75), (20, 72), "3 pixels below the rung snaps onto it");
    assert_eq!(sky.snap_pointer(20, 70), (20, 70), "on the rung stays put");
    assert_eq!(sky.snap_pointer(20, 79), (20, 79), "out of reach stays put");
}
