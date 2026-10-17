//! The walk grids: one bit per 8x8 block of each room, set where something
//! stands. Port of ScummVM's engines/sky/grid.cpp.

use super::defs::*;
use super::sky::{f, Sky};

const GRID_FILE_START: u16 = 60000;
const TOT: usize = TOT_NO_GRIDS as usize;

/// Room number to grid number; -1 for rooms without one. 70 (TOT_NO_GRIDS)
/// appears for two rooms in the original and is out of range there too.
const GRID_CONVERT: [i8; 97] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29,
    30, 31, 32, 33, 34, -1, 35, 36, 37, 38, 39, 40, 41, -1, 42, 43, 44, 45, 46, -1, -1, -1, -1, -1, -1, -1, -1, -1,
    -1, -1, -1, -1, -1, -1, -1, 47, 70, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, -1, 61, 62, -1, -1, -1,
    -1, -1, -1, 70, 63, 64, 65, 66, 67, 68, 69,
];

#[derive(Default)]
pub struct Grid {
    pub grids: Vec<Option<Vec<u8>>>,
}

fn grid_index(screen: u32) -> Option<usize> {
    let g = *GRID_CONVERT.get(screen as usize)?;
    if g >= 0 && (g as usize) < TOT {
        Some(g as usize)
    } else {
        None
    }
}

impl Sky {
    pub(crate) fn load_grids(&mut self) {
        self.grid.grids = (0..TOT).map(|i| self.disk.load(GRID_FILE_START + i as u16)).collect();
        if !self.disk.is_demo() && self.vars[REICH_DOOR_FLAG as usize] != 0 {
            // reloading the grids would put the open Reich door's grid bit
            // back, locking Foster in (or out)
            let door = self.cpt_start(CPT_REICH_DOOR_20);
            self.remove_grid(256, 280, 1, door);
        }
    }

    pub fn give_grid(&self, screen: u32) -> Option<&Vec<u8>> {
        grid_index(screen).and_then(|g| self.grid.grids.get(g)).and_then(|g| g.as_ref())
    }

    fn grid_values(&self, x: u32, y: u32, width: u32, c: usize) -> Option<(usize, u32, u32)> {
        if y < TOP_LEFT_Y {
            return None; // off screen
        }
        let y = (y - TOP_LEFT_Y) >> 3;
        if y >= GAME_SCREEN_HEIGHT >> 3 {
            return None;
        }
        let mut bit_pos = y * 40;
        let mut width = width + 1;
        let mut x = x >> 3;
        if x < TOP_LEFT_X >> 3 {
            // at least partly off screen
            if x + width < TOP_LEFT_X >> 3 {
                return None;
            }
            width -= (TOP_LEFT_X >> 3) - x;
            x = 0;
        } else {
            x -= TOP_LEFT_X >> 3;
        }
        if GAME_SCREEN_WIDTH >> 3 <= x {
            return None;
        }
        if GAME_SCREEN_WIDTH >> 3 < x + width {
            width = (GAME_SCREEN_WIDTH >> 3) - x;
        }
        bit_pos += x;
        let grid = grid_index(self.g(c, f::SCREEN) as u32)?;
        let tmp = 0x1F - (bit_pos & 0x1F);
        bit_pos &= !0x1F; // dword address and bit number
        bit_pos += tmp;
        Some((grid, bit_pos, width))
    }

    fn set_grid_bits(&mut self, grid: usize, mut bit: u32, width: u32, on: bool) {
        let Some(Some(g)) = self.grid.grids.get_mut(grid) else { return };
        for _ in 0..width {
            if let Some(b) = g.get_mut((bit >> 3) as usize) {
                if on {
                    *b |= 1 << (bit & 7);
                } else {
                    *b &= !(1 << (bit & 7));
                }
            }
            if bit & 0x1F == 0 {
                bit += 0x3F;
            } else {
                bit -= 1;
            }
        }
    }

    pub(crate) fn remove_object_from_walk(&mut self, c: usize) {
        let width = self.w(self.mega_set(c) + f::MS_GRID_WIDTH) as u32;
        let (x, y) = (self.g(c, f::XCOOD) as u32, self.g(c, f::YCOOD) as u32);
        if let Some((g, bit, w)) = self.grid_values(x, y, width, c) {
            self.set_grid_bits(g, bit, w, false);
        }
    }

    pub(crate) fn object_to_walk(&mut self, c: usize) {
        let width = self.w(self.mega_set(c) + f::MS_GRID_WIDTH) as u32;
        let (x, y) = (self.g(c, f::XCOOD) as u32, self.g(c, f::YCOOD) as u32);
        if let Some((g, bit, w)) = self.grid_values(x, y, width, c) {
            self.set_grid_bits(g, bit, w, true);
        }
    }

    pub(crate) fn plot_grid(&mut self, x: u32, y: u32, width: u32, c: usize) {
        if let Some((g, bit, w)) = self.grid_values(x, y, width.wrapping_sub(1), c) {
            self.set_grid_bits(g, bit, w, true);
        }
    }

    pub(crate) fn remove_grid(&mut self, x: u32, y: u32, width: u32, c: usize) {
        if let Some((g, bit, w)) = self.grid_values(x, y, width, c) {
            self.set_grid_bits(g, bit, w, false);
        }
    }
}
