//! Which sprite and screen files are in memory. Port of the caching half
//! of ScummVM's engines/sky/disk.cpp (fnCacheChip, fnCacheFast, ...).
//! Files live in `Sky::items`, indexed by file number & 2047.

use super::sky::Sky;

impl Sky {
    fn load_item(&mut self, file: u16) {
        let data = self.disk.load(file & 0x7FFF);
        if data.is_none() {
            log::warn!("fnCacheFiles: file {} did not load", file & 0x7FFF);
        }
        self.items[(file & 2047) as usize] = data;
    }

    /// fnCacheChip: called after fnCacheFast; appends a list and loads.
    pub(crate) fn disk_cache_chip(&mut self, mut list: usize) {
        let mut cnt = self.build_list.iter().position(|&f| f == 0).unwrap_or(self.build_list.len());
        loop {
            let v = self.w(list);
            list += 1;
            if cnt < self.build_list.len() {
                self.build_list[cnt] = v & 0x7FFF;
            }
            cnt += 1;
            if v == 0 {
                break;
            }
        }
        self.disk_cache_files();
    }

    pub(crate) fn disk_cache_fast(&mut self, list: Option<usize>) {
        if let Some(mut list) = list {
            let mut cnt = 0;
            loop {
                let v = self.w(list);
                list += 1;
                if cnt < self.build_list.len() {
                    self.build_list[cnt] = v & 0x7FFF;
                }
                cnt += 1;
                if v == 0 {
                    break;
                }
            }
        }
    }

    fn disk_cache_files(&mut self) {
        // keep what is still wanted, drop the rest
        let mut targ = 0usize;
        let mut l = 0usize;
        while self.loaded_files[l] != 0 {
            let lf = self.loaded_files[l];
            let wanted = self.build_list.iter().take_while(|&&b| b != 0).any(|&b| (b & 0x7FFF) as u32 == lf);
            if wanted {
                self.loaded_files[targ] = lf;
                targ += 1;
            } else {
                self.items[(lf & 2047) as usize] = None;
            }
            l += 1;
        }
        self.loaded_files[targ] = 0;

        let mut b = 0usize;
        while self.build_list[b] != 0 {
            let want = self.build_list[b];
            if want & 0x7FF == 0x7FF {
                // amiga dummy files
                b += 1;
                continue;
            }
            let loaded = self.loaded_files.iter().take_while(|&&f| f != 0).any(|&f| f == (want & 0x7FFF) as u32);
            if !loaded {
                self.loaded_files[targ] = (want & 0x7FFF) as u32;
                targ += 1;
                self.loaded_files[targ] = 0;
                self.load_item(want);
            }
            b += 1;
        }
        self.build_list[0] = 0;
    }

    /// Disk::refreshFilesList, used on restore.
    pub(crate) fn refresh_files_list(&mut self, list: &[u32]) {
        let mut cnt = 0;
        while self.loaded_files[cnt] != 0 {
            self.items[(self.loaded_files[cnt] & 2047) as usize] = None;
            cnt += 1;
        }
        let mut cnt = 0;
        while cnt < list.len() && list[cnt] != 0 {
            self.loaded_files[cnt] = list[cnt];
            self.load_item((list[cnt] & 0x7FFF) as u16);
            cnt += 1;
        }
        self.loaded_files[cnt] = 0;
    }

    pub(crate) fn fn_mini_load(&mut self, file: u16) {
        let mut cnt = 0;
        while self.loaded_files[cnt] != 0 {
            if self.loaded_files[cnt] == file as u32 {
                return;
            }
            cnt += 1;
        }
        self.loaded_files[cnt] = (file & 0x7FFF) as u32;
        self.loaded_files[cnt + 1] = 0;
        self.items[(file & 2047) as usize] = self.disk.load(file);
    }

    pub(crate) fn fn_flush_buffers(&mut self) {
        // dump all loaded sprites
        let mut l = 0;
        while self.loaded_files[l] != 0 {
            self.items[(self.loaded_files[l] & 2047) as usize] = None;
            l += 1;
        }
        self.loaded_files[0] = 0;
    }

    /// The loaded files list, as saved in save games.
    pub fn loaded_files_list(&self) -> Vec<u32> {
        self.loaded_files.iter().take_while(|&&f| f != 0).copied().collect()
    }
}
