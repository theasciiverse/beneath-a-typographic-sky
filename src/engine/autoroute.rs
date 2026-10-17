//! Path finding over the walk grid. Port of ScummVM's
//! engines/sky/autoroute.cpp.

use super::defs::*;
use super::sky::{f, Sky};

const RGW: usize = (GAME_SCREEN_WIDTH / 8) as usize + 2;
const RGH: usize = (GAME_SCREEN_HEIGHT / 8) as usize + 2;
const WALK_JUMP: u16 = 8;
const ROUTE_WORDS: usize = (ROUTE_SPACE / 2) as usize;

const ROUTE_DIRECTIONS: [isize; 4] = [-1, 1, -(RGW as isize), RGW as isize];
const LOGIC_COMMANDS: [u16; 4] = [RIGHTY as u16, LEFTY as u16, DOWNY as u16, UPY as u16];

pub struct RouteState {
    grid: Vec<u16>,
    buf: Vec<u16>,
}

impl Default for RouteState {
    fn default() -> Self {
        RouteState { grid: vec![0; RGW * RGH], buf: vec![0; ROUTE_WORDS] }
    }
}

fn clip_x(x: u16) -> (u8, i16) {
    let (x, tl) = (x as i32, TOP_LEFT_X as i32);
    if x < tl {
        (0, (x - tl) as i16)
    } else if x >= tl + GAME_SCREEN_WIDTH as i32 {
        (((GAME_SCREEN_WIDTH - 1) >> 3) as u8, (x - (tl + GAME_SCREEN_WIDTH as i32 - 1)) as i16)
    } else {
        (((x - tl) >> 3) as u8, 0)
    }
}

fn clip_y(y: u16) -> (u8, i16) {
    let (y, tl) = (y as i32, TOP_LEFT_Y as i32);
    if y < tl {
        (0, (y - tl) as i16)
    } else if y >= tl + GAME_SCREEN_HEIGHT as i32 {
        (((GAME_SCREEN_HEIGHT - 1) >> 3) as u8, (y - (tl + GAME_SCREEN_HEIGHT as i32)) as i16)
    } else {
        (((y - tl) >> 3) as u8, 0)
    }
}

impl RouteState {
    fn check_block(&self, pos: usize) -> u16 {
        let mut ret = 0xFFFF;
        for d in ROUTE_DIRECTIONS {
            let v = self.grid[(pos as isize + d) as usize];
            if v != 0 && v < ret {
                ret = v;
            }
        }
        ret
    }

    fn init_walk_grid(&mut self, screen_grid: &[u8], width: u8) {
        self.grid.iter_mut().for_each(|v| *v = 0);
        let mut src = super::defs::GRID_SIZE as usize; // read backwards from the end
        let mut pos = RGW * RGH - RGW - 2;
        let mut bits_left = 0u8;
        let mut data = 0u32;
        for _ in 0..RGH - 2 {
            let mut stretch = 0u8;
            for _ in 0..RGW - 2 {
                if bits_left == 0 {
                    src -= 4;
                    data = u32::from_le_bytes([
                        screen_grid[src],
                        screen_grid[src + 1],
                        screen_grid[src + 2],
                        screen_grid[src + 3],
                    ]);
                    bits_left = 32;
                }
                if data & 1 != 0 {
                    self.grid[pos] = 0xFFFF; // not accessible
                    stretch = width;
                } else if stretch != 0 {
                    self.grid[pos] = 0xFFFF;
                    stretch -= 1;
                }
                pos -= 1;
                bits_left -= 1;
                data >>= 1;
            }
            pos -= 2;
        }
    }

    fn calc_walk_grid(&mut self, sx: u8, sy: u8, dx: u8, dy: u8) -> bool {
        let (dir_y, mut roi_y): (isize, u8) =
            if sy > dy { (-(RGW as isize), sy) } else { (RGW as isize, (RGH - 1) as u8 - sy) };
        let (dir_x, mut roi_x): (isize, u8) =
            if sx > dx { (-1, sx + 2) } else { (1, (RGW - 1) as u8 - sx) };

        let dest = (dy as usize + 1) * RGW + dx as usize + 1;
        let mut start = ((sy as usize + 1) * RGW + sx as usize + 1) as isize;
        self.grid[start as usize] = 1;

        // on the edge: move diagonally from the start
        if (roi_y as usize) < RGH - 3 {
            start -= dir_y;
        }
        if (roi_x as usize) < RGW - 2 {
            start -= dir_x;
        }

        let mut changed = true;
        let mut found = false;
        while !found && changed {
            changed = false;
            let mut yp = start;
            for _ in 0..roi_y {
                let mut xp = yp;
                for _ in 0..roi_x {
                    let i = xp as usize;
                    if self.grid[i] == 0 {
                        let r = self.check_block(i);
                        if r < 0xFFFF {
                            self.grid[i] = r + 1;
                            changed = true;
                        }
                    }
                    xp += dir_x;
                }
                yp += dir_y;
            }
            if self.grid[dest] != 0 {
                found = true;
            } else {
                // extend the rectangle of interest
                if (roi_y as usize) < RGH - 4 {
                    start -= dir_y;
                    roi_y += 1;
                }
                if (roi_x as usize) < RGW - 4 {
                    start -= dir_x;
                    roi_x += 1;
                }
            }
        }
        found
    }

    /// Writes the route backwards from the end of `buf`; returns the index
    /// of its first word pair.
    fn make_route_data(&mut self, dx: u8, dy: u8) -> usize {
        self.buf.iter_mut().for_each(|v| *v = 0);
        let mut pos = ((dy as usize + 1) * RGW + dx as usize + 1) as isize;
        let mut trg = ROUTE_WORDS - 2;
        let mut last = self.grid[pos as usize].wrapping_sub(1);
        while last != 0 {
            trg -= 2;
            let mut walk = 0isize;
            for i in 0..4 {
                if last == self.grid[(pos + ROUTE_DIRECTIONS[i]) as usize] {
                    self.buf[trg + 1] = LOGIC_COMMANDS[i];
                    walk = ROUTE_DIRECTIONS[i];
                    break;
                }
            }
            assert!(walk != 0, "makeRouteData: can't find way through walkGrid (pos {last})");
            while last != 0 && last == self.grid[(pos + walk) as usize] {
                self.buf[trg] += WALK_JUMP;
                last -= 1;
                pos += walk;
            }
        }
        trg
    }

    fn check_init_move(&mut self, mut at: usize, init_x: i16) -> usize {
        if init_x < 0 {
            at -= 2;
            self.buf[at + 1] = RIGHTY as u16;
            self.buf[at] = (((-init_x) as u16) + 7) & 0xFFF8;
        } else if init_x > 0 {
            at -= 2;
            self.buf[at + 1] = LEFTY as u16;
            self.buf[at] = ((init_x as u16) + 7) & 0xFFF8;
        }
        at
    }
}

impl Sky {
    /// AutoRoute::autoRoute: 0 route made, 1 no route, 2 already there.
    pub(crate) fn auto_route(&mut self, c: usize) -> u16 {
        let screen = self.g(c, f::SCREEN) as u8;
        let width = self.w(self.mega_set(c) + f::MS_GRID_WIDTH) as u8;
        let Some(grid) = self.give_grid(screen as u32).cloned() else {
            log::warn!("autoRoute: no grid for screen {screen}");
            return 1;
        };
        self.route.init_walk_grid(&grid, width);

        let (sx, init_sx) = clip_x(self.g(c, f::XCOOD));
        let (sy, _) = clip_y(self.g(c, f::YCOOD));
        let (dx, _) = clip_x(self.g(c, f::AR_TARGET_X));
        let (dy, _) = clip_y(self.g(c, f::AR_TARGET_Y));

        let dest = self.cpt_start(self.g(c, f::ANIM_SCRATCH_ID) as u32);
        for i in 0..32 {
            self.sw(dest + i, 0);
        }
        if sx == dx && sy == dy {
            return 2;
        }

        let di = (dy as usize + 1) * RGW + dx as usize + 1;
        if self.route.grid[di] != 0 {
            if self.cpt_is_id(c, CPT_FOSTER) && screen == 12 && dx == 2 && dy == 14 {
                // workaround for script bug #1804: in the pipe factory Joey
                // can block Foster's target when Lamb throws him out, and the
                // game would loop forever; hide that part of Joey
                self.route.grid[di] = 0;
            } else {
                return 1; // destination is not accessible
            }
        }

        if !self.route.calc_walk_grid(sx, sy, dx, dy) {
            return 1; // no route to the block
        }
        let start = self.route.make_route_data(dx, dy);
        // an initial x move (due to clipping) goes on the front
        let start = self.route.check_init_move(start, init_sx);

        let mut i = 0usize;
        loop {
            let (a, b) = (self.route.buf[start + i], self.route.buf[start + i + 1]);
            self.sw(dest + i, a);
            self.sw(dest + i + 1, b);
            i += 2;
            if a == 0 {
                break;
            }
        }
        0
    }
}
