//! Phase 0 circuit solver spike (plan sections 8 and 13). Throwaway.
//!
//! The model follows section 8 as far as it is specific, and fills gaps with
//! the simplest choice (each is noted). Wire is a 6-connected grid block with
//! strength 0–15 that falls by 1 per block. Connected wire compiles into nets.
//! Inverters and repeaters read the wire behind them and drive the wire in
//! front, always delayed by at least one logic tick, through a timing wheel.
//! Sources drive every adjacent wire at 15. Lamps light when any adjacent wire
//! is above 0. Component outputs are binary (0 or 15).
//!
//! Not modelled: solid blocks carrying power, comparators, observers, pistons,
//! chunk borders and loading, parallel solving.

pub mod naive;

/// Block position `[x, y, z]`.
pub type Pos = [i32; 3];

/// Unit steps for direction indices 0..6: +X, -X, +Y, -Y, +Z, -Z.
/// `d ^ 1` is the opposite direction.
pub const DIRS: [Pos; 6] = [
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
];

/// Adds direction `d` to a position.
pub fn step_pos(p: Pos, d: usize) -> Pos {
    [p[0] + DIRS[d][0], p[1] + DIRS[d][1], p[2] + DIRS[d][2]]
}

/// A circuit block as placed by a builder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    Wire,
    /// Constant power source (a lever); drives all adjacent wire.
    Source {
        on: bool,
    },
    /// Reads the wire behind it, drives the wire in front with the opposite state, delay 1.
    Inverter {
        facing: u8,
    },
    /// Reads the wire behind it, drives the wire in front with the same state, after `delay` ticks.
    Repeater {
        facing: u8,
        delay: u8,
    },
    /// Lit when any adjacent wire has strength above 0.
    Lamp,
}

const NONE: u32 = u32::MAX;
const WHEEL: usize = 8;
const MAX_LEVEL: usize = 15;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Air,
    Wire,
    Source,
    Inverter,
    Repeater,
    Lamp,
}

#[derive(Clone, Copy)]
struct Cell {
    kind: Kind,
    facing: u8,
    delay: u8,
    comp: u32,
}

const AIR: Cell = Cell {
    kind: Kind::Air,
    facing: 0,
    delay: 0,
    comp: NONE,
};

struct Comp {
    cell: u32,
    /// Current output: 15 or 0. For a lamp, 15 when lit.
    out: u8,
    /// Last value scheduled (gates only), so unchanged inputs schedule nothing.
    target: u8,
}

#[derive(Default)]
struct Net {
    cells: Vec<u32>,
    /// `(wire cell, component)` pairs: a component whose output feeds that cell.
    drivers: Vec<(u32, u32)>,
    /// Cells with strength above 0.
    lit: Vec<u32>,
    dirty: bool,
    alive: bool,
}

/// What one logic tick did.
#[derive(Default, Clone, Copy, Debug)]
pub struct StepStats {
    /// Components whose output (or lamp state) changed.
    pub switched: usize,
    pub nets_solved: usize,
    pub wire_cells_changed: usize,
}

/// The event-driven solver.
pub struct Sim {
    dims: [i32; 3],
    px: i32,
    off: [isize; 6],
    cells: Vec<Cell>,
    level: Vec<u8>,
    net: Vec<u32>,
    comps: Vec<Comp>,
    nets: Vec<Net>,
    free_nets: Vec<u32>,
    dirty: Vec<u32>,
    changed: Vec<u32>,
    wheel: [Vec<(u32, u8)>; WHEEL],
    tick: u32,
    // Scratch, reused across solves so the hot path doesn't allocate.
    scratch: Vec<u8>,
    touched: Vec<u32>,
    buckets: Vec<Vec<u32>>,
    stack: Vec<u32>,
}

impl Sim {
    /// Builds a world of `dims` blocks and places `blocks` in it. Positions
    /// outside `dims` are ignored. Initial state is settled and all gates are
    /// evaluated once, as if the circuit had just loaded.
    pub fn new(dims: [i32; 3], blocks: &[(Pos, Block)]) -> Self {
        // One block of air padding on every side, so neighbour lookups never leave the array.
        let (px, py, pz) = (dims[0] + 2, dims[1] + 2, dims[2] + 2);
        let len = (px * py * pz) as usize;
        let (sx, sy) = (px as isize, (px * py) as isize);
        let mut sim = Self {
            dims,
            px,
            off: [1, -1, sx, -sx, sy, -sy],
            cells: vec![AIR; len],
            level: vec![0; len],
            net: vec![NONE; len],
            comps: Vec::new(),
            nets: Vec::new(),
            free_nets: Vec::new(),
            dirty: Vec::new(),
            changed: Vec::new(),
            wheel: Default::default(),
            tick: 0,
            scratch: vec![0; len],
            touched: Vec::new(),
            buckets: vec![Vec::new(); MAX_LEVEL + 1],
            stack: Vec::new(),
        };
        for &(p, b) in blocks {
            if let Some(c) = sim.index(p) {
                sim.place(c, b);
            }
        }
        for c in 0..len as u32 {
            if sim.cells[c as usize].kind == Kind::Wire && sim.net[c as usize] == NONE {
                sim.label_from(c);
            }
        }
        sim.solve_dirty();
        // Initial evaluation of every reader.
        for comp in 0..sim.comps.len() as u32 {
            sim.evaluate(comp);
        }
        sim.changed.clear();
        sim
    }

    fn index(&self, p: Pos) -> Option<u32> {
        let ok = (0..3).all(|i| p[i] >= 0 && p[i] < self.dims[i]);
        ok.then(|| {
            let py = self.dims[1] + 2;
            ((p[0] + 1) + self.px * ((p[1] + 1) + py * (p[2] + 1))) as u32
        })
    }

    fn pos_of(&self, c: u32) -> Pos {
        let (c, px, py) = (c as i32, self.px, self.dims[1] + 2);
        [c % px - 1, (c / px) % py - 1, c / (px * py) - 1]
    }

    #[inline]
    fn nb(&self, c: u32, d: usize) -> u32 {
        (c as isize + self.off[d]) as u32
    }

    fn place(&mut self, c: u32, b: Block) {
        let comp = self.comps.len() as u32;
        let (kind, facing, delay, out) = match b {
            Block::Wire => (Kind::Wire, 0, 0, 0),
            Block::Source { on } => (Kind::Source, 0, 0, if on { 15 } else { 0 }),
            Block::Inverter { facing } => (Kind::Inverter, facing, 1, 0),
            Block::Repeater { facing, delay } => (Kind::Repeater, facing, delay.clamp(1, 4), 0),
            Block::Lamp => (Kind::Lamp, 0, 0, 0),
        };
        if kind == Kind::Wire {
            self.cells[c as usize] = Cell { kind, ..AIR };
            return;
        }
        self.cells[c as usize] = Cell {
            kind,
            facing,
            delay,
            comp,
        };
        self.comps.push(Comp {
            cell: c,
            out,
            target: 0,
        });
    }

    /// Number of components (everything except wire).
    pub fn component_count(&self) -> usize {
        self.comps.len()
    }

    /// Live nets.
    pub fn net_count(&self) -> usize {
        self.nets.iter().filter(|n| n.alive).count()
    }

    /// Wire strength at `p`, or 0 where there is no wire.
    pub fn level_at(&self, p: Pos) -> u8 {
        self.index(p).map_or(0, |c| self.level[c as usize])
    }

    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// Output of every component, sorted by position. Lamps report 15 when lit.
    pub fn snapshot(&self) -> Vec<(Pos, u8)> {
        let mut v: Vec<_> = self
            .comps
            .iter()
            .map(|c| (self.pos_of(c.cell), c.out))
            .collect();
        v.sort_unstable();
        v
    }

    // ---- nets -------------------------------------------------------------

    fn alloc_net(&mut self) -> u32 {
        if let Some(id) = self.free_nets.pop() {
            self.nets[id as usize].alive = true;
            id
        } else {
            self.nets.push(Net {
                alive: true,
                ..Net::default()
            });
            self.nets.len() as u32 - 1
        }
    }

    fn mark_dirty(&mut self, id: u32) {
        let n = &mut self.nets[id as usize];
        if !n.dirty {
            n.dirty = true;
            self.dirty.push(id);
        }
    }

    /// Flood-labels the unlabelled wire reachable from `seed` as one new net.
    fn label_from(&mut self, seed: u32) {
        let id = self.alloc_net();
        let mut net = std::mem::take(&mut self.nets[id as usize]);
        net.alive = true;
        self.net[seed as usize] = id;
        self.stack.push(seed);
        while let Some(c) = self.stack.pop() {
            net.cells.push(c);
            for d in 0..6 {
                let n = self.nb(c, d);
                if self.cells[n as usize].kind == Kind::Wire && self.net[n as usize] == NONE {
                    self.net[n as usize] = id;
                    self.stack.push(n);
                }
            }
        }
        for &c in &net.cells {
            if self.level[c as usize] > 0 {
                net.lit.push(c);
            }
            for d in 0..6 {
                let q = self.nb(c, d);
                let cell = self.cells[q as usize];
                let drives = match cell.kind {
                    Kind::Source => true,
                    Kind::Inverter | Kind::Repeater => self.nb(q, cell.facing as usize) == c,
                    _ => false,
                };
                if drives {
                    net.drivers.push((c, cell.comp));
                }
            }
        }
        self.nets[id as usize] = net;
        self.mark_dirty(id);
    }

    /// Places or removes one wire block (a no-op on any other block), rebuilding
    /// only the nets it touches. Cost is proportional to the size of those nets.
    pub fn set_wire(&mut self, p: Pos, on: bool) {
        let Some(c) = self.index(p) else { return };
        let kind = self.cells[c as usize].kind;
        if (on && kind != Kind::Air) || (!on && kind != Kind::Wire) {
            return;
        }
        // Dissolve every net the edit can merge or split.
        let mut region: Vec<u32> = Vec::new();
        let mut touched_nets: Vec<u32> = Vec::with_capacity(7);
        if !on {
            touched_nets.push(self.net[c as usize]);
        }
        for d in 0..6 {
            let n = self.net[self.nb(c, d) as usize];
            if n != NONE && !touched_nets.contains(&n) {
                touched_nets.push(n);
            }
        }
        for id in touched_nets {
            let net = std::mem::take(&mut self.nets[id as usize]);
            for &cell in &net.cells {
                self.net[cell as usize] = NONE;
            }
            region.extend_from_slice(&net.cells);
            self.free_nets.push(id);
            // A recycled slot starts clean; a stale entry in `dirty` is skipped by its flag.
            self.nets[id as usize] = Net::default();
        }
        if on {
            self.cells[c as usize] = Cell {
                kind: Kind::Wire,
                ..AIR
            };
            region.push(c);
        } else {
            self.cells[c as usize] = AIR;
            self.net[c as usize] = NONE;
            if self.level[c as usize] != 0 {
                self.level[c as usize] = 0;
                self.changed.push(c);
            }
            region.retain(|&x| x != c);
        }
        for cell in region {
            if self.net[cell as usize] == NONE && self.cells[cell as usize].kind == Kind::Wire {
                self.label_from(cell);
            }
        }
    }

    /// Flips a source, if `p` is one.
    pub fn toggle_source(&mut self, p: Pos) {
        let Some(c) = self.index(p) else { return };
        let cell = self.cells[c as usize];
        if cell.kind != Kind::Source {
            return;
        }
        let comp = &mut self.comps[cell.comp as usize];
        comp.out = if comp.out == 0 { 15 } else { 0 };
        self.mark_outputs_dirty(cell.comp);
    }

    fn mark_outputs_dirty(&mut self, comp: u32) {
        let cell_idx = self.comps[comp as usize].cell;
        let cell = self.cells[cell_idx as usize];
        match cell.kind {
            Kind::Source => {
                for d in 0..6 {
                    let n = self.nb(cell_idx, d);
                    let id = self.net[n as usize];
                    if id != NONE {
                        self.mark_dirty(id);
                    }
                }
            }
            Kind::Inverter | Kind::Repeater => {
                let n = self.nb(cell_idx, cell.facing as usize);
                let id = self.net[n as usize];
                if id != NONE {
                    self.mark_dirty(id);
                }
            }
            _ => {}
        }
    }

    // ---- solving ----------------------------------------------------------

    /// Recomputes one net's strengths from its active drivers (a multi-source
    /// flood in 15 buckets) and records which cells changed.
    fn solve(&mut self, id: u32) {
        let net = std::mem::take(&mut self.nets[id as usize]);
        let mut buckets = std::mem::take(&mut self.buckets);
        let mut touched = std::mem::take(&mut self.touched);
        touched.clear();
        for &(cell, comp) in &net.drivers {
            let s = self.comps[comp as usize].out;
            if s > 0 && self.scratch[cell as usize] < s {
                if self.scratch[cell as usize] == 0 {
                    touched.push(cell);
                }
                self.scratch[cell as usize] = s;
                buckets[s as usize].push(cell);
            }
        }
        for s in (2..=MAX_LEVEL).rev() {
            while let Some(c) = buckets[s].pop() {
                if self.scratch[c as usize] as usize != s {
                    continue; // superseded by a stronger path
                }
                for d in 0..6 {
                    let n = self.nb(c, d);
                    if self.cells[n as usize].kind == Kind::Wire
                        && (self.scratch[n as usize] as usize) < s - 1
                    {
                        if self.scratch[n as usize] == 0 {
                            touched.push(n);
                        }
                        self.scratch[n as usize] = (s - 1) as u8;
                        buckets[s - 1].push(n);
                    }
                }
            }
        }
        buckets[1].clear();
        for &c in &touched {
            let new = self.scratch[c as usize];
            if self.level[c as usize] != new {
                self.level[c as usize] = new;
                self.changed.push(c);
            }
        }
        for &c in &net.lit {
            if self.scratch[c as usize] == 0 && self.level[c as usize] != 0 {
                self.level[c as usize] = 0;
                self.changed.push(c);
            }
        }
        for &c in &touched {
            self.scratch[c as usize] = 0;
        }
        let mut net = net;
        net.lit.clear();
        net.lit.extend_from_slice(&touched);
        self.nets[id as usize] = net;
        self.buckets = buckets;
        self.touched = touched;
    }

    fn solve_dirty(&mut self) -> usize {
        let dirty = std::mem::take(&mut self.dirty);
        let mut solved = 0;
        for &id in &dirty {
            let n = &mut self.nets[id as usize];
            if !n.dirty || !n.alive {
                continue;
            }
            n.dirty = false;
            self.solve(id);
            solved += 1;
        }
        let mut dirty = dirty;
        dirty.clear();
        self.dirty = dirty;
        solved
    }

    fn input_level(&self, cell_idx: u32, facing: u8) -> u8 {
        self.level[self.nb(cell_idx, (facing ^ 1) as usize) as usize]
    }

    /// Re-reads a component's inputs. Gates schedule an output change only if
    /// the desired value differs from the last one scheduled. Lamps follow immediately.
    /// Returns true if a lamp changed state.
    fn evaluate(&mut self, comp: u32) -> bool {
        let cell_idx = self.comps[comp as usize].cell;
        let cell = self.cells[cell_idx as usize];
        match cell.kind {
            Kind::Inverter | Kind::Repeater => {
                let input = self.input_level(cell_idx, cell.facing);
                let on = (input > 0) == (cell.kind == Kind::Repeater);
                let desired = if on { 15 } else { 0 };
                let c = &mut self.comps[comp as usize];
                if desired != c.target {
                    c.target = desired;
                    let slot = (self.tick as usize + cell.delay as usize) % WHEEL;
                    self.wheel[slot].push((comp, desired));
                }
                false
            }
            Kind::Lamp => {
                let lit = (0..6).any(|d| self.level[self.nb(cell_idx, d) as usize] > 0);
                let c = &mut self.comps[comp as usize];
                let new = if lit { 15 } else { 0 };
                let changed = c.out != new;
                c.out = new;
                changed
            }
            _ => false,
        }
    }

    /// Advances one logic tick: apply due output changes, solve dirty nets,
    /// then let readers of changed wire react (scheduling into the wheel).
    pub fn step(&mut self) -> StepStats {
        self.tick += 1;
        let mut stats = StepStats::default();
        let slot = self.tick as usize % WHEEL;
        let mut due = std::mem::take(&mut self.wheel[slot]);
        for &(comp, value) in &due {
            if self.comps[comp as usize].out != value {
                self.comps[comp as usize].out = value;
                stats.switched += 1;
                self.mark_outputs_dirty(comp);
            }
        }
        due.clear();
        self.wheel[slot] = due;

        stats.nets_solved = self.solve_dirty();

        let mut changed = std::mem::take(&mut self.changed);
        stats.wire_cells_changed = changed.len();
        for &c in &changed {
            for d in 0..6 {
                let q = self.nb(c, d);
                let cell = self.cells[q as usize];
                let reads = match cell.kind {
                    Kind::Inverter | Kind::Repeater => cell.facing as usize == d,
                    Kind::Lamp => true,
                    _ => false,
                };
                if reads && self.evaluate(cell.comp) {
                    stats.switched += 1;
                }
            }
        }
        changed.clear();
        self.changed = changed;
        stats
    }
}
