//! Slow reference solver: every tick, recompute all wire strengths from
//! scratch by relaxing to a fixed point, and re-read every component. It
//! shares no solving code with `Sim` (only the `Block` and direction
//! definitions), so a bug in net tracking or incremental updates shows up as
//! a trace mismatch.

use std::collections::BTreeMap;

use crate::{Block, Pos, step_pos};

pub struct Naive {
    blocks: BTreeMap<Pos, Block>,
    /// Output of each component (lamps: 15 when lit).
    out: BTreeMap<Pos, u8>,
    /// Last value scheduled, per gate.
    target: BTreeMap<Pos, u8>,
    /// `(due tick, position, value)` in scheduling order.
    pending: Vec<(u32, Pos, u8)>,
    level: BTreeMap<Pos, u8>,
    dims: [i32; 3],
    tick: u32,
}

impl Naive {
    pub fn new(dims: [i32; 3], blocks: &[(Pos, Block)]) -> Self {
        let mut n = Self {
            blocks: BTreeMap::new(),
            out: BTreeMap::new(),
            target: BTreeMap::new(),
            pending: Vec::new(),
            level: BTreeMap::new(),
            dims,
            tick: 0,
        };
        for &(p, b) in blocks {
            if n.inside(p) {
                n.blocks.insert(p, b);
                if b != Block::Wire {
                    n.out.insert(
                        p,
                        if b == (Block::Source { on: true }) {
                            15
                        } else {
                            0
                        },
                    );
                    n.target.insert(p, 0);
                }
            }
        }
        n.recompute_levels();
        n.evaluate_all();
        n
    }

    fn inside(&self, p: Pos) -> bool {
        (0..3).all(|i| p[i] >= 0 && p[i] < self.dims[i])
    }

    pub fn set_wire(&mut self, p: Pos, on: bool) {
        if !self.inside(p) {
            return;
        }
        match (self.blocks.get(&p), on) {
            (None, true) => {
                self.blocks.insert(p, Block::Wire);
            }
            (Some(Block::Wire), false) => {
                self.blocks.remove(&p);
            }
            _ => {}
        }
    }

    pub fn toggle_source(&mut self, p: Pos) {
        if let Some(Block::Source { .. }) = self.blocks.get(&p) {
            let o = self.out.get_mut(&p).expect("source has an output");
            *o = if *o == 0 { 15 } else { 0 };
        }
    }

    pub fn tick(&self) -> u32 {
        self.tick
    }

    pub fn level_at(&self, p: Pos) -> u8 {
        self.level.get(&p).copied().unwrap_or(0)
    }

    pub fn snapshot(&self) -> Vec<(Pos, u8)> {
        self.out.iter().map(|(&p, &o)| (p, o)).collect()
    }

    pub fn step(&mut self) {
        self.tick += 1;
        let tick = self.tick;
        let mut keep = Vec::new();
        for (due, p, v) in std::mem::take(&mut self.pending) {
            if due == tick {
                self.out.insert(p, v);
            } else {
                keep.push((due, p, v));
            }
        }
        self.pending = keep;
        self.recompute_levels();
        self.evaluate_all();
    }

    fn recompute_levels(&mut self) {
        self.level.clear();
        for (&p, &b) in &self.blocks {
            if b == Block::Wire {
                self.level.insert(p, 0);
            }
        }
        // Seed from every active driver.
        let mut seeds: Vec<(Pos, u8)> = Vec::new();
        for (&p, &b) in &self.blocks {
            if b == Block::Wire {
                continue;
            }
            let o = self.out[&p];
            if o == 0 {
                continue;
            }
            match b {
                Block::Source { .. } => (0..6).for_each(|d| seeds.push((step_pos(p, d), o))),
                Block::Inverter { facing } | Block::Repeater { facing, .. } => {
                    seeds.push((step_pos(p, facing as usize), o))
                }
                _ => {}
            }
        }
        for (cell, s) in seeds {
            if let Some(l) = self.level.get_mut(&cell) {
                *l = (*l).max(s);
            }
        }
        // Relax until nothing changes: each wire is at least its strongest neighbour minus one.
        loop {
            let mut changed = false;
            let cells: Vec<Pos> = self.level.keys().copied().collect();
            for p in cells {
                let best = (0..6)
                    .filter_map(|d| self.level.get(&step_pos(p, d)).copied())
                    .max()
                    .unwrap_or(0)
                    .saturating_sub(1);
                if best > self.level[&p] {
                    self.level.insert(p, best);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn evaluate_all(&mut self) {
        let blocks: Vec<(Pos, Block)> = self.blocks.iter().map(|(&p, &b)| (p, b)).collect();
        for (p, b) in blocks {
            match b {
                Block::Inverter { facing } | Block::Repeater { facing, .. } => {
                    let input = self.level_at(step_pos(p, (facing ^ 1) as usize));
                    let inverter = matches!(b, Block::Inverter { .. });
                    let desired = if (input > 0) != inverter { 15 } else { 0 };
                    if desired != self.target[&p] {
                        self.target.insert(p, desired);
                        let delay = match b {
                            Block::Repeater { delay, .. } => u32::from(delay.clamp(1, 4)),
                            _ => 1,
                        };
                        self.pending.push((self.tick + delay, p, desired));
                    }
                }
                Block::Lamp => {
                    let lit = (0..6).any(|d| self.level_at(step_pos(p, d)) > 0);
                    self.out.insert(p, if lit { 15 } else { 0 });
                }
                _ => {}
            }
        }
    }
}
