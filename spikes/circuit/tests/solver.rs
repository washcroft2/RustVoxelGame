//! Golden circuits, a comparison against the slow reference solver on random
//! circuits (with random edits), and invariance under translation and rotation
//! (plan section 8: "Testing").

use proptest::prelude::*;
use spike_circuit::naive::Naive;
use spike_circuit::{Block, Pos, Sim};

/// Minimal xorshift so circuits are reproducible from a seed.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn wires(from: Pos, len: i32) -> Vec<(Pos, Block)> {
    (0..len)
        .map(|i| ([from[0] + i, from[1], from[2]], Block::Wire))
        .collect()
}

fn run(sim: &mut Sim, ticks: usize) -> Vec<Vec<(Pos, u8)>> {
    (0..ticks)
        .map(|_| {
            sim.step();
            sim.snapshot()
        })
        .collect()
}

fn out_at(snap: &[(Pos, u8)], p: Pos) -> u8 {
    snap.iter()
        .find(|(q, _)| *q == p)
        .expect("component present")
        .1
}

#[test]
fn strength_falls_by_one_per_block_and_stops_at_zero() {
    let mut blocks = vec![([0, 0, 0], Block::Source { on: true })];
    blocks.extend(wires([1, 0, 0], 20));
    let sim = Sim::new([30, 3, 3], &blocks);
    assert_eq!(sim.level_at([1, 0, 0]), 15);
    assert_eq!(sim.level_at([8, 0, 0]), 8);
    assert_eq!(sim.level_at([15, 0, 0]), 1);
    assert_eq!(sim.level_at([16, 0, 0]), 0);
    assert_eq!(sim.level_at([20, 0, 0]), 0);
}

#[test]
fn wire_takes_the_stronger_of_two_sources() {
    let mut blocks = vec![
        ([0, 0, 0], Block::Source { on: true }),
        ([11, 0, 0], Block::Source { on: true }),
    ];
    blocks.extend(wires([1, 0, 0], 10));
    let sim = Sim::new([14, 3, 3], &blocks);
    assert_eq!(sim.level_at([1, 0, 0]), 15);
    assert_eq!(sim.level_at([10, 0, 0]), 15);
    // Cell 5 is 4 blocks from the first source's wire and 5 from the second's; cell 6 the reverse.
    assert_eq!(sim.level_at([5, 0, 0]), 11);
    assert_eq!(sim.level_at([6, 0, 0]), 11);
}

#[test]
fn inverter_loop_is_a_period_two_clock() {
    // Inverter at (1,0,0) facing +X reads (0,0,0), drives (2,0,0); wire loops back around.
    let blocks = vec![
        ([1, 0, 0], Block::Inverter { facing: 0 }),
        ([2, 0, 0], Block::Wire),
        ([2, 0, 1], Block::Wire),
        ([1, 0, 1], Block::Wire),
        ([0, 0, 1], Block::Wire),
        ([0, 0, 0], Block::Wire),
    ];
    let mut sim = Sim::new([6, 2, 3], &blocks);
    let trace: Vec<u8> = run(&mut sim, 8)
        .iter()
        .map(|s| out_at(s, [1, 0, 0]))
        .collect();
    assert_eq!(trace, [15, 0, 15, 0, 15, 0, 15, 0]);
}

#[test]
fn three_inverter_ring_oscillates() {
    // A square loop of wire with an inverter in three of its eight cells:
    //   (0,2) W  (1,2) I<-  (2,2) W
    //   (0,1) W             (2,1) I^
    //   (0,0) W  (1,0) I->  (2,0) W
    // Each inverter reads the wire before it and drives the wire after it.
    let blocks = vec![
        ([1, 0, 0], Block::Inverter { facing: 0 }), // reads (0,0), drives (2,0)
        ([2, 0, 1], Block::Inverter { facing: 4 }), // reads (2,0), drives (2,2)
        ([1, 0, 2], Block::Inverter { facing: 1 }), // reads (2,2), drives (0,2)
        ([2, 0, 0], Block::Wire),
        ([2, 0, 2], Block::Wire),
        ([0, 0, 2], Block::Wire),
        ([0, 0, 1], Block::Wire),
        ([0, 0, 0], Block::Wire),
    ];
    let mut sim = Sim::new([3, 1, 3], &blocks);
    assert_eq!(sim.net_count(), 3);
    let mut reference = Naive::new([3, 1, 3], &blocks);
    let mut first = Vec::new();
    for _ in 0..12 {
        sim.step();
        reference.step();
        assert_eq!(sim.snapshot(), reference.snapshot());
        first.push(out_at(&sim.snapshot(), [1, 0, 0]));
    }
    // All three start together, so the ring flips as one: period two, and never stuck.
    assert_eq!(first, [15, 0, 15, 0, 15, 0, 15, 0, 15, 0, 15, 0]);
}

#[test]
fn repeater_delay_is_exact() {
    for delay in 1..=4u8 {
        let blocks = vec![
            ([0, 0, 0], Block::Source { on: false }),
            ([1, 0, 0], Block::Wire),
            ([2, 0, 0], Block::Repeater { facing: 0, delay }),
            ([3, 0, 0], Block::Wire),
            ([4, 0, 0], Block::Lamp),
        ];
        let mut sim = Sim::new([6, 2, 2], &blocks);
        run(&mut sim, 3);
        sim.toggle_source([0, 0, 0]);
        let trace: Vec<u8> = run(&mut sim, 8)
            .iter()
            .map(|s| out_at(s, [4, 0, 0]))
            .collect();
        let first_lit = trace.iter().position(|&v| v == 15).expect("lamp lights") as u8 + 1;
        // Tick 1 evaluates the repeater; its output lands `delay` ticks later, the lamp follows at once.
        assert_eq!(first_lit, 1 + delay, "delay {delay}: trace {trace:?}");
    }
}

#[test]
fn edits_split_and_merge_nets() {
    let mut blocks = vec![([0, 0, 0], Block::Source { on: true })];
    blocks.extend(wires([1, 0, 0], 10));
    blocks.push(([11, 0, 0], Block::Lamp));
    let mut sim = Sim::new([14, 3, 3], &blocks);
    sim.step();
    assert_eq!(out_at(&sim.snapshot(), [11, 0, 0]), 15);
    assert_eq!(sim.net_count(), 1);
    sim.set_wire([5, 0, 0], false); // cut
    sim.step();
    assert_eq!(sim.net_count(), 2);
    assert_eq!(out_at(&sim.snapshot(), [11, 0, 0]), 0);
    sim.set_wire([5, 0, 0], true); // heal
    sim.step();
    assert_eq!(sim.net_count(), 1);
    assert_eq!(out_at(&sim.snapshot(), [11, 0, 0]), 15);
}

// ---- random circuits -----------------------------------------------------

const DIMS: [i32; 3] = [10, 3, 10];

fn random_circuit(seed: u64, dims: [i32; 3]) -> Vec<(Pos, Block)> {
    let mut rng = Rng(seed | 1);
    let mut cells: std::collections::BTreeMap<Pos, Block> = std::collections::BTreeMap::new();
    for z in 0..dims[2] {
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                let r = rng.below(100);
                let facing = rng.below(6) as u8;
                let block = match r {
                    0..=39 => Block::Wire,
                    40..=42 => Block::Source {
                        on: rng.below(2) == 0,
                    },
                    43..=52 => Block::Inverter { facing },
                    53..=58 => Block::Repeater {
                        facing,
                        delay: 1 + rng.below(4) as u8,
                    },
                    59..=62 => Block::Lamp,
                    _ => continue,
                };
                cells.insert([x, y, z], block);
            }
        }
    }
    // Stamp a few self-oscillating inverter loops, each with a short wire tap pointing +X,
    // into a cleared halo. Left to chance, random neighbours merge into the loop's net and
    // hold it constant, and almost nothing would oscillate. The tap lets the clock drive
    // whatever random components lie beyond it.
    for _ in 0..1 + rng.below(3) {
        let (x, y, z) = (
            rng.below((dims[0] - 7).max(1) as u64) as i32,
            rng.below(dims[1] as u64) as i32,
            rng.below((dims[2] - 2).max(1) as u64) as i32,
        );
        let tap = 1 + rng.below(4) as i32;
        for hz in z - 1..=z + 2 {
            for hy in y - 1..=y + 1 {
                for hx in x - 1..=x + 2 {
                    cells.remove(&[hx, hy, hz]);
                }
            }
        }
        let mut stamp = vec![
            ([x + 1, y, z], Block::Inverter { facing: 0 }),
            ([x + 2, y, z], Block::Wire),
            ([x + 2, y, z + 1], Block::Wire),
            ([x + 1, y, z + 1], Block::Wire),
            ([x, y, z + 1], Block::Wire),
            ([x, y, z], Block::Wire),
        ];
        stamp.extend((0..tap).map(|i| ([x + 3 + i, y, z], Block::Wire)));
        for (p, b) in stamp {
            cells.insert(p, b);
        }
    }
    cells.into_iter().collect()
}

#[derive(Clone, Copy, Debug)]
enum Edit {
    Wire(Pos, bool),
    Toggle(Pos),
}

fn random_edits(seed: u64, ticks: usize, dims: [i32; 3]) -> Vec<(usize, Edit)> {
    let mut rng = Rng(seed.wrapping_mul(31) | 1);
    let mut v = Vec::new();
    for t in 0..ticks {
        if rng.below(100) < 30 {
            let p = [
                rng.below(dims[0] as u64) as i32,
                rng.below(dims[1] as u64) as i32,
                rng.below(dims[2] as u64) as i32,
            ];
            v.push((
                t,
                match rng.below(3) {
                    0 => Edit::Wire(p, true),
                    1 => Edit::Wire(p, false),
                    _ => Edit::Toggle(p),
                },
            ));
        }
    }
    v
}

fn compare(seed: u64, ticks: usize, with_edits: bool) {
    let blocks = random_circuit(seed, DIMS);
    let edits = if with_edits {
        random_edits(seed, ticks, DIMS)
    } else {
        Vec::new()
    };
    let mut sim = Sim::new(DIMS, &blocks);
    let mut reference = Naive::new(DIMS, &blocks);
    assert_eq!(
        sim.snapshot(),
        reference.snapshot(),
        "initial state, seed {seed}"
    );
    for t in 0..ticks {
        for &(_, e) in edits.iter().filter(|(et, _)| *et == t) {
            match e {
                Edit::Wire(p, on) => {
                    sim.set_wire(p, on);
                    reference.set_wire(p, on);
                }
                Edit::Toggle(p) => {
                    sim.toggle_source(p);
                    reference.toggle_source(p);
                }
            }
        }
        sim.step();
        reference.step();
        assert_eq!(
            sim.snapshot(),
            reference.snapshot(),
            "seed {seed}, tick {}",
            t + 1
        );
    }
}

#[test]
fn fixed_seeds_match_reference() {
    for seed in 1..=40 {
        compare(seed, 60, false);
        compare(seed, 60, true);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn random_circuits_match_reference(seed in any::<u64>()) {
        compare(seed, 50, false);
    }

    #[test]
    fn random_circuits_with_edits_match_reference(seed in any::<u64>()) {
        compare(seed, 50, true);
    }

    /// Moving or turning a circuit, including across 32-block borders, must not change what it does.
    #[test]
    fn translation_and_rotation_do_not_change_behaviour(
        seed in any::<u64>(),
        ox in 0i32..70, oy in 0i32..70, oz in 0i32..70,
        turns in 0u8..4,
    ) {
        let base = random_circuit(seed, DIMS);
        let big = [DIMS[0] + 80, DIMS[1] + 80, DIMS[2] + 80];

        let turn = |p: Pos| -> Pos {
            let mut p = p;
            for _ in 0..turns {
                p = [DIMS[2] - 1 - p[2], p[1], p[0]]; // quarter turn about Y, staying in the box
            }
            p
        };
        let turn_dir = |d: u8| -> u8 {
            let mut d = d;
            for _ in 0..turns {
                d = [4, 5, 2, 3, 1, 0][d as usize]; // +X->+Z, -X->-Z, +Z->-X, -Z->+X
            }
            d
        };
        let moved: Vec<(Pos, Block)> = base
            .iter()
            .map(|&(p, b)| {
                let q = turn(p);
                let b = match b {
                    Block::Inverter { facing } => Block::Inverter { facing: turn_dir(facing) },
                    Block::Repeater { facing, delay } => Block::Repeater { facing: turn_dir(facing), delay },
                    other => other,
                };
                ([q[0] + ox, q[1] + oy, q[2] + oz], b)
            })
            .collect();

        let mut a = Sim::new(DIMS, &base);
        let mut b = Sim::new(big, &moved);
        for t in 0..40 {
            a.step();
            b.step();
            let mut want: Vec<(Pos, u8)> = a
                .snapshot()
                .into_iter()
                .map(|(p, v)| {
                    let q = turn(p);
                    ([q[0] + ox, q[1] + oy, q[2] + oz], v)
                })
                .collect();
            want.sort_unstable();
            prop_assert_eq!(want, b.snapshot(), "tick {}", t + 1);
        }
    }
}

#[test]
fn random_circuits_are_not_trivially_inert() {
    // Guards the comparison tests: if the generator stopped producing sustained activity
    // they would pass vacuously. Count circuits still changing in their last 20 ticks.
    // Observed when written: 10/40 circuits and 693 changing ticks; thresholds are about 60% of that.
    let mut still_active = 0;
    let mut changing_ticks = 0;
    for seed in 1..=40 {
        let mut sim = Sim::new(DIMS, &random_circuit(seed, DIMS));
        let mut last = sim.snapshot();
        let mut late = false;
        for t in 0..60 {
            sim.step();
            let now = sim.snapshot();
            if now != last {
                changing_ticks += 1;
                late |= t >= 40;
            }
            last = now;
        }
        still_active += usize::from(late);
    }
    assert!(
        still_active >= 6,
        "only {still_active}/40 circuits still active at the end"
    );
    assert!(
        changing_ticks >= 400,
        "only {changing_ticks} ticks with changes"
    );
}
