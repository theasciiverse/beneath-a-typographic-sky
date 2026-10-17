//! Restart, save, restore and the options panel. Restart and the save
//! format are a port of ScummVM's engines/sky/control.cpp, so save files
//! are the same as ScummVM's SKY-VM.nnn. The panel itself is the
//! terminal's own (a menu the host draws), not the game's pixel panel.

use std::fs;
use std::path::PathBuf;

use super::defs::*;
use super::host::MenuItem;
use super::sky::{Sky, NUM_SKY_SCRIPTVARS};

const SAVE_FILE_REVISION: u32 = 6;
pub const SAVE_SLOTS: u16 = 20;

impl Sky {
    /// Restarts the game, skipping the intro: Foster on the top level of
    /// the factory.
    pub fn restart_game(&mut self) {
        if self.sys.game_version <= 267 {
            return; // no restart for the floppy demo
        }
        let Some(words) = self.cpt.reset_data(self.sys.game_version) else {
            panic!("no reset data for version {}", self.sys.game_version);
        };
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        if let Err(e) = self.parse_save_data(&bytes) {
            panic!("restart failed: {e}");
        }
        self.sys.seen_screen = [false; super::sky::TOTAL_SCREENS];
        self.force_refresh();
        self.screen.current.iter_mut().for_each(|p| *p = 0);
        self.show_current();
        let pal = self.cpt_bytes(self.sys.current_palette);
        self.set_palette(&pal);
        let m = self.mouse.current_cursor;
        self.sprite_mouse(m, 0, 0);
        self.sys.past_intro = true;
    }

    /// Control::prepareSaveData.
    pub fn prepare_save_data(&self) -> Vec<u8> {
        let mut d: Vec<u8> = vec![0; 4]; // space for the size
        let d32 = |d: &mut Vec<u8>, v: u32| d.extend_from_slice(&v.to_le_bytes());
        let d16 = |d: &mut Vec<u8>, v: u16| d.extend_from_slice(&v.to_le_bytes());
        d32(&mut d, SAVE_FILE_REVISION);
        d32(&mut d, self.sys.game_version);
        let saved_sounds = self.audio_save_sounds();
        d16(&mut d, saved_sounds[0]);
        d16(&mut d, saved_sounds[1]);
        d32(&mut d, self.sys.current_music as u32);
        d32(&mut d, self.text.cur_char_set);
        d32(&mut d, self.mouse.current_cursor as u32);
        d32(&mut d, self.sys.current_palette);
        for v in self.vars.iter() {
            d32(&mut d, *v);
        }
        for i in 0..60 {
            d32(&mut d, self.loaded_files[i]);
        }
        for &id in &self.cpt.save_ids {
            let e = self.cpt.entry(id).expect("saved compact exists");
            for w in &self.cpt.raw[e.start..e.start + e.size as usize] {
                d16(&mut d, *w);
            }
        }
        let size = d.len() as u32;
        d[..4].copy_from_slice(&size.to_le_bytes());
        d
    }

    /// Control::parseSaveData.
    pub fn parse_save_data(&mut self, src: &[u8]) -> Result<(), String> {
        let mut p = 0usize;
        let rd32 = |p: &mut usize| -> Result<u32, String> {
            let b = src.get(*p..*p + 4).ok_or("save data truncated")?;
            *p += 4;
            Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let rd16 = |p: &mut usize| -> Result<u16, String> {
            let b = src.get(*p..*p + 2).ok_or("save data truncated")?;
            *p += 2;
            Ok(u16::from_le_bytes([b[0], b[1]]))
        };
        let size = rd32(&mut p)?;
        let rev = rd32(&mut p)?;
        if rev != SAVE_FILE_REVISION {
            return Err(format!("unsupported save file revision {rev}"));
        }
        let version = rd32(&mut p)?;
        if version != self.sys.game_version && (!self.disk.is_cd() || version < 365) {
            return Err(format!(
                "saved by Beneath a Steel Sky v0.0{version:03}, cannot load in v0.0{:03}",
                self.sys.game_version
            ));
        }
        self.sys.system_flags |= SF_GAME_RESTORED;
        let s0 = rd16(&mut p)?;
        let s1 = rd16(&mut p)?;
        self.audio_restore_sfx([s0, s1]);
        let music = rd32(&mut p)?;
        let char_set = rd32(&mut p)?;
        let mouse_type = rd32(&mut p)?;
        let palette = rd32(&mut p)?;

        let mut vars = vec![0u32; NUM_SKY_SCRIPTVARS];
        for v in vars.iter_mut() {
            *v = rd32(&mut p)?;
        }
        self.parse_save_vars(&vars);

        let mut reload = vec![0u32; 60];
        for v in reload.iter_mut() {
            *v = rd32(&mut p)?;
        }
        for i in 0..self.cpt.save_ids.len() {
            let id = self.cpt.save_ids[i];
            let e = self.cpt.entry(id).expect("saved compact exists");
            for w in 0..e.size as usize {
                self.cpt.raw[e.start + w] = rd16(&mut p)?;
            }
        }
        // make sure all text compacts are off
        for id in CPT_TEXT_1..=CPT_TEXT_11 {
            let c = self.cpt_start(id);
            self.s(c, super::sky::f::STATUS, 0);
        }
        if p != size as usize {
            return Err(format!("save data is {p} bytes, expected {size}"));
        }
        self.refresh_files_list(&reload);
        self.sys.current_music = music as u16;
        if self.sys.system_flags & SF_MUS_OFF == 0 {
            self.audio_start_music(music as u16);
        }
        self.fn_set_font(char_set.min(2));
        self.mouse.current_cursor = mouse_type as u16;
        self.sys.current_palette = palette;
        Ok(())
    }

    pub fn save_dir() -> PathBuf {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("typographic-sky")
    }

    fn slot_path(slot: u16) -> PathBuf {
        Self::save_dir().join(format!("SKY-VM.{slot:03}"))
    }

    fn descriptions() -> Vec<String> {
        let text = fs::read_to_string(Self::save_dir().join("descriptions.txt")).unwrap_or_default();
        let mut d: Vec<String> = text.lines().map(str::to_string).collect();
        d.resize(SAVE_SLOTS as usize, String::new());
        d
    }

    pub fn save_game(&mut self, slot: u16, description: &str) -> Result<(), String> {
        fs::create_dir_all(Self::save_dir()).map_err(|e| e.to_string())?;
        let data = self.prepare_save_data();
        fs::write(Self::slot_path(slot), data).map_err(|e| e.to_string())?;
        let mut d = Self::descriptions();
        if slot >= 1 && slot <= SAVE_SLOTS {
            d[slot as usize - 1] = description.replace('\n', " ");
            fs::write(Self::save_dir().join("descriptions.txt"), d.join("\n")).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn restore_game(&mut self, slot: u16) -> Result<(), String> {
        let data = fs::read(Self::slot_path(slot)).map_err(|e| e.to_string())?;
        self.parse_save_data(&data)?;
        self.sys.past_intro = true;
        self.screen.current.iter_mut().for_each(|p| *p = 0);
        self.show_current();
        self.force_refresh();
        let pal = self.cpt_bytes(self.sys.current_palette);
        self.set_palette(&pal);
        let m = self.mouse.current_cursor;
        self.sprite_mouse(m, 0, 0);
        Ok(())
    }

    fn can_save(&self) -> bool {
        // as ScummVM: not while choosing dialogue or before the intro ends
        self.sys.system_flags & SF_CHOOSING == 0 && self.sys.past_intro
    }

    /// The options panel (F5).
    pub fn do_control_panel(&mut self) {
        loop {
            let text_on = self.sys.system_flags & SF_ALLOW_TEXT != 0;
            let speech_on = self.sys.system_flags & SF_ALLOW_SPEECH != 0;
            let items = vec![
                MenuItem::new("Restore a game"),
                MenuItem::new("Save the game").enabled(self.can_save()),
                MenuItem::new(if text_on { "Subtitles: on" } else { "Subtitles: off" }),
                MenuItem::new(if speech_on { "Speech: on" } else { "Speech: off" }).enabled(self.disk.is_cd()),
                MenuItem::new("Restart"),
                MenuItem::new("Quit"),
                MenuItem::new("Resume"),
            ];
            match self.host.menu("Beneath a Typographic Sky", &items) {
                Some(0) => {
                    if self.restore_menu() {
                        return;
                    }
                }
                Some(1) => {
                    self.save_menu();
                    return;
                }
                Some(2) => {
                    // keep at least one of text and speech
                    if !text_on || speech_on {
                        self.sys.system_flags ^= SF_ALLOW_TEXT;
                    }
                }
                Some(3) => {
                    if !speech_on || text_on {
                        self.sys.system_flags ^= SF_ALLOW_SPEECH;
                    }
                }
                Some(4) => {
                    if self.host.confirm("Restart the game?") {
                        self.restart_game();
                        return;
                    }
                }
                Some(5) => {
                    if self.host.confirm("Quit the game?") {
                        self.quit = true;
                        return;
                    }
                }
                _ => return,
            }
        }
    }

    fn slot_items(&self, for_save: bool) -> Vec<MenuItem> {
        let d = Self::descriptions();
        (1..=SAVE_SLOTS)
            .map(|s| {
                let exists = Self::slot_path(s).exists();
                let name = if exists && !d[s as usize - 1].is_empty() {
                    d[s as usize - 1].clone()
                } else if exists {
                    "(saved game)".to_string()
                } else {
                    "(empty)".to_string()
                };
                MenuItem::new(&format!("{s:2}  {name}")).enabled(for_save || exists)
            })
            .collect()
    }

    /// Returns true when a game was restored.
    fn restore_menu(&mut self) -> bool {
        let items = self.slot_items(false);
        if let Some(i) = self.host.menu("Restore a game", &items) {
            match self.restore_game(i as u16 + 1) {
                Ok(()) => return true,
                Err(e) => self.host.message(&format!("Restore failed: {e}")),
            }
        }
        false
    }

    fn save_menu(&mut self) {
        let items = self.slot_items(true);
        if let Some(i) = self.host.menu("Save the game", &items) {
            let slot = i as u16 + 1;
            let old = Self::descriptions()[i].clone();
            if let Some(name) = self.host.text_input("Name this save", &old) {
                if let Err(e) = self.save_game(slot, &name) {
                    self.host.message(&format!("Save failed: {e}"));
                }
            }
        }
    }

    /// fnRestoreGame: the restore panel the game opens itself (after a death).
    pub fn do_load_save_panel(&mut self) {
        loop {
            let items = vec![MenuItem::new("Restore a game"), MenuItem::new("Restart"), MenuItem::new("Quit")];
            match self.host.menu("Game over", &items) {
                Some(0) => {
                    if self.restore_menu() {
                        return;
                    }
                }
                Some(1) => {
                    self.restart_game();
                    return;
                }
                Some(2) => {
                    self.quit = true;
                    return;
                }
                _ => {}
            }
            if self.quit {
                return;
            }
        }
    }
}
