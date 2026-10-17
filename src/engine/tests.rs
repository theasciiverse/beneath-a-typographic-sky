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
