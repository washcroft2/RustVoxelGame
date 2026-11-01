//! Circuit solver benchmark. Run: `cargo run --release -p spike-circuit`.
//!
//! 1. The plan's target: 100k components, 10% switching per logic tick, in 5 ms.
//! 2. A stress case the plan doesn't mention: one huge net with many drivers.
//! 3. The cost of editing wire (net rebuild) as a function of net size.

use std::time::{Duration, Instant};

use spike_circuit::{Block, Pos, Sim};

fn main() {
    workload_many_small_circuits();
    workload_huge_net();
    edit_costs();
}

/// 10,000 units of 10 components each. Every tenth unit is a live clock whose
/// signal ripples down a chain of repeaters to a lamp, so every component in
/// it switches every tick; the rest are cut loops that settle and stay quiet.
fn workload_many_small_circuits() {
    println!("== 1. many small circuits: 10,000 units x 10 components");
    let dims = [640, 5, 480];
    let mut blocks: Vec<(Pos, Block)> = Vec::new();
    let mut unit = 0usize;
    let per_row = 30; // 21 wide each: one spare column so neighbouring units never touch
    'fill: for layer in 0..3 {
        for row in 0..160 {
            for col in 0..per_row {
                if unit == 10_000 {
                    break 'fill;
                }
                let active = unit.is_multiple_of(10);
                stamp_unit(&mut blocks, [col * 21, layer * 2, row * 3], active);
                unit += 1;
            }
        }
    }
    let t = Instant::now();
    let mut sim = Sim::new(dims, &blocks);
    println!(
        "build: {:?}  components {}  nets {}  blocks placed {}",
        t.elapsed(),
        sim.component_count(),
        sim.net_count(),
        blocks.len()
    );
    for _ in 0..200 {
        sim.step();
    }
    let mut times = Vec::new();
    let (mut switched, mut solved, mut cells) = (0usize, 0usize, 0usize);
    for _ in 0..1000 {
        let t = Instant::now();
        let s = sim.step();
        times.push(t.elapsed());
        switched += s.switched;
        solved += s.nets_solved;
        cells += s.wire_cells_changed;
    }
    times.sort();
    println!(
        "per tick: {:.0} components switched ({:.1}% of {}), {:.0} nets solved, {:.0} wire cells changed",
        switched as f64 / 1000.0,
        100.0 * switched as f64 / 1000.0 / sim.component_count() as f64,
        sim.component_count(),
        solved as f64 / 1000.0,
        cells as f64 / 1000.0
    );
    report("step time", &times);
    println!("target (section 2): <= 5 ms");
    rss();
}

/// One 20-wide unit (plus a spare column) along +X at `o`: an inverter loop (cut when idle) feeding
/// eight repeaters in series and a lamp.
fn stamp_unit(blocks: &mut Vec<(Pos, Block)>, o: Pos, active: bool) {
    let at = |dx: i32, dz: i32| [o[0] + dx, o[1], o[2] + dz];
    blocks.push((at(1, 0), Block::Inverter { facing: 0 }));
    for (dx, dz) in [(0, 0), (2, 0), (2, 1), (1, 1)] {
        blocks.push((at(dx, dz), Block::Wire));
    }
    if active {
        blocks.push((at(0, 1), Block::Wire)); // closes the loop
    }
    for i in 0..8 {
        blocks.push((
            at(3 + 2 * i, 0),
            Block::Repeater {
                facing: 0,
                delay: 1,
            },
        ));
        blocks.push((at(4 + 2 * i, 0), Block::Wire));
    }
    blocks.push((at(19, 0), Block::Lamp));
}

/// One 100,000-cell wire with 10,000 sources along it. Each tick toggles 1% of
/// the sources, so the single net is dirty every tick.
fn workload_huge_net() {
    println!("\n== 2. one huge net: 100,000 wire cells, 10,000 sources, 100 toggled per tick");
    let n = 100_000;
    let mut blocks: Vec<(Pos, Block)> = (0..n).map(|x| ([x, 0, 1], Block::Wire)).collect();
    let sources: Vec<Pos> = (0..n).step_by(10).map(|x| [x, 0, 0]).collect();
    blocks.extend(sources.iter().map(|&p| (p, Block::Source { on: true })));
    let mut sim = Sim::new([n, 1, 2], &blocks);
    let mut rng = 0x1234_5678_9ABC_DEF1u64;
    let mut times = Vec::new();
    let mut changed = 0usize;
    for _ in 0..300 {
        for _ in 0..100 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            sim.toggle_source(sources[(rng % sources.len() as u64) as usize]);
        }
        let t = Instant::now();
        let s = sim.step();
        times.push(t.elapsed());
        changed += s.wire_cells_changed;
    }
    times.sort();
    report("step time", &times);
    println!(
        "wire cells changing per tick: {:.0}",
        changed as f64 / 300.0
    );
    println!("(no section 2 target for this case; it shows the cost of one big dirty net)");
}

/// Placing and removing wire in a straight net of each size.
fn edit_costs() {
    println!("\n== 3. editing wire in a straight net (net rebuild cost)");
    println!(
        "{:<10} {:>14} {:>14} {:>14} {:>14}",
        "net size", "extend end", "remove end", "cut middle", "heal middle"
    );
    for size in [100, 1_000, 10_000, 100_000] {
        // Source at x = 0, wire at x = 1..=size, so both ends of the wire are free to edit.
        let mut blocks: Vec<(Pos, Block)> = (1..=size).map(|x| ([x, 0, 0], Block::Wire)).collect();
        blocks.push(([0, 0, 0], Block::Source { on: true }));
        let mut sim = Sim::new([size + 3, 1, 1], &blocks);
        let mid = [size / 2, 0, 0];
        let beyond = [size + 1, 0, 0];
        let reps = if size >= 100_000 { 5 } else { 20 };
        let (mut a, mut b, mut c, mut d) = (
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
        );
        for _ in 0..reps {
            let t = Instant::now();
            sim.set_wire(beyond, true);
            a += t.elapsed();
            sim.step();
            let t = Instant::now();
            sim.set_wire(beyond, false);
            b += t.elapsed();
            sim.step();
            let t = Instant::now();
            sim.set_wire(mid, false);
            c += t.elapsed();
            sim.step();
            let t = Instant::now();
            sim.set_wire(mid, true);
            d += t.elapsed();
            sim.step();
        }
        let f = |x: Duration| format!("{:.1} µs", x.as_nanos() as f64 / 1000.0 / reps as f64);
        println!(
            "{:<10} {:>14} {:>14} {:>14} {:>14}",
            size,
            f(a),
            f(b),
            f(c),
            f(d)
        );
    }
    println!("(edit call only; the solve that follows happens in the next step)");
}

fn report(label: &str, sorted: &[Duration]) {
    let p = |q: usize| sorted[(sorted.len() * q / 100).min(sorted.len() - 1)];
    println!(
        "{label}: median {:.3} ms  p99 {:.3} ms  max {:.3} ms",
        p(50).as_secs_f64() * 1e3,
        p(99).as_secs_f64() * 1e3,
        sorted[sorted.len() - 1].as_secs_f64() * 1e3
    );
}

/// Resident memory of this process, Linux only.
fn rss() {
    if let Ok(s) = std::fs::read_to_string("/proc/self/statm")
        && let Some(pages) = s
            .split_whitespace()
            .nth(1)
            .and_then(|p| p.parse::<u64>().ok())
    {
        println!("resident memory: {} MB", pages * 4096 / 1_000_000);
    }
}
