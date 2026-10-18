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
        self.restore_data(&data)
    }

    /// Restores a game from save data, as from a slot, and redraws.
    pub fn restore_data(&mut self, data: &[u8]) -> Result<(), String> {
        self.parse_save_data(data)?;
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

    /// ScummVM's canSaveGameStateCurrently and Control::loadSaveAllowed.
    pub(crate) fn can_save(&self) -> bool {
        let screen = self.vars[SCREEN as usize];
        self.sys.past_intro
            // texts get lost during load/save, so not while choosing
            && self.sys.system_flags & SF_CHOOSING == 0
            // the same problem with LINC terminals
            && screen < 101
            // not in the final rooms
            && !(screen >= 82 && screen != 85 && screen < 90)
    }

    /// Safe to autosave: saving allowed, not the death screen, and no game
    /// text up (a restore switches text off, so a LINC panel saved open
    /// would come back empty).
    fn autosave_safe(&self) -> bool {
        self.can_save()
            && self.vars[SCREEN as usize] != 102
            && (CPT_TEXT_1..=CPT_TEXT_11).all(|id| self.g(self.cpt_start(id), super::sky::f::STATUS) == 0)
    }

    /// Called every game cycle by the terminal game: remembers the last
    /// moment the autosave could safely be taken.
    pub(crate) fn note_safe_moment(&mut self) {
        if self.front_end && self.autosave_safe() {
            self.last_safe = Some(self.prepare_save_data());
        }
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
                MenuItem::new(&format!("Language: {}", LANGUAGES[self.sys.language as usize].1)),
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
                Some(4) => self.language_menu(),
                Some(5) => {
                    if self.host.confirm("Restart the game?") {
                        self.restart_game();
                        return;
                    }
                }
                Some(6) => {
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

    /// fnRestoreGame: the game's own Restore button (on the death screen)
    /// opens the restore list; leaving it returns to the game.
    pub fn do_load_save_panel(&mut self) {
        self.restore_menu();
    }

    // ----- our additions: start menu and autosave ----------------------

    fn intro_seen_path() -> PathBuf {
        Self::save_dir().join("intro-seen")
    }

    fn mark_intro_seen() {
        let _ = fs::create_dir_all(Self::save_dir());
        let _ = fs::write(Self::intro_seen_path(), "");
    }

    /// How the terminal game starts: the intro the first time, a start
    /// menu after that. Tests and tools call `start` directly.
    pub fn begin(&mut self, skip_intro: bool) {
        if !self.front_end || skip_intro {
            self.start(skip_intro);
            return;
        }
        if !Self::intro_seen_path().exists() {
            self.start(false);
            Self::mark_intro_seen();
            return;
        }
        loop {
            let has_auto = Self::slot_path(0).exists();
            let has_saves = (1..=SAVE_SLOTS).any(|s| Self::slot_path(s).exists());
            let items = vec![
                MenuItem::new("Continue").enabled(has_auto),
                MenuItem::new("Restore a game").enabled(has_saves),
                MenuItem::new("New game"),
                MenuItem::new("Watch the intro"),
                MenuItem::new("Quit"),
            ];
            match self.host.menu("Beneath a Typographic Sky", &items) {
                Some(0) => match self.restore_game(0) {
                    Ok(()) => {
                        log::debug!("start menu: continue");
                        return;
                    }
                    Err(e) => self.host.message(&format!("Could not continue: {e}")),
                },
                Some(1) => {
                    if self.restore_menu() {
                        return;
                    }
                }
                Some(2) => {
                    self.start(true);
                    return;
                }
                Some(3) => {
                    self.start(false);
                    return;
                }
                Some(4) => {
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

    /// Saves to ScummVM's autosave slot (0) when the game is left, so the
    /// start menu can continue from there. Not on the death screen, and
    /// not where the game itself forbids saving.
    pub(crate) fn autosave_on_quit(&mut self) {
        if !self.front_end {
            return;
        }
        // now if it is safe, else the last safe moment, else not at all
        let data = if self.autosave_safe() { Some(self.prepare_save_data()) } else { self.last_safe.take() };
        let Some(data) = data else {
            log::debug!("no safe moment to autosave");
            return;
        };
        let result = fs::create_dir_all(Self::save_dir()).and_then(|_| fs::write(Self::slot_path(0), data));
        match result {
            Ok(()) => log::debug!("autosaved"),
            Err(e) => log::warn!("autosave failed: {e}"),
        }
    }

    // ----- our addition: the language ---------------------------------

    fn language_path() -> PathBuf {
        Self::save_dir().join("language")
    }

    /// The terminal game's language at start: the one chosen in the menu
    /// before, else the system's (LC_ALL, LC_MESSAGES, LANG), else
    /// English (David, 2026-10-18).
    pub fn init_language(&mut self) {
        let saved = fs::read_to_string(Self::language_path()).ok();
        let code = saved.as_deref().map(str::trim).map(str::to_string).or_else(|| {
            ["LC_ALL", "LC_MESSAGES", "LANG"]
                .iter()
                .filter_map(|v| std::env::var(v).ok())
                .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
        });
        let lang = code.as_deref().map(language_from_locale).unwrap_or(SKY_ENGLISH as u16);
        if !self.set_language(lang) {
            self.set_language(SKY_ENGLISH as u16);
        }
        log::debug!("language {} from {:?}", LANGUAGES[self.sys.language as usize].1, code);
    }

    fn language_menu(&mut self) {
        let items: Vec<MenuItem> = LANGUAGES
            .iter()
            .map(|&(n, name, _)| MenuItem::new(name).enabled(self.disk.exists(60600 + n * 8)))
            .collect();
        if let Some(i) = self.host.menu("Language (speech stays English)", &items) {
            let (n, _, code) = LANGUAGES[i];
            if self.set_language(n) {
                let _ = fs::create_dir_all(Self::save_dir());
                let _ = fs::write(Self::language_path(), code);
            }
        }
    }
}

/// The game's languages: number, own name, the locale code we store.
const LANGUAGES: [(u16, &str, &str); 8] = [
    (0, "English", "en_GB"),
    (1, "Deutsch", "de"),
    (2, "Français", "fr"),
    (3, "English (US)", "en_US"),
    (4, "Svenska", "sv"),
    (5, "Italiano", "it"),
    (6, "Português", "pt"),
    (7, "Español", "es"),
];

/// A locale such as sv_SE.UTF-8 to the game's language number.
fn language_from_locale(loc: &str) -> u16 {
    let l = loc.to_ascii_lowercase();
    let prefix = |p: &str| l.starts_with(p);
    if prefix("en_us") {
        3
    } else if prefix("de") {
        1
    } else if prefix("fr") {
        2
    } else if prefix("sv") {
        4
    } else if prefix("it") {
        5
    } else if prefix("pt") {
        6
    } else if prefix("es") {
        7
    } else {
        0
    }
}
