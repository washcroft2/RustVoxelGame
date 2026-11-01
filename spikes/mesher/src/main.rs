//! Mesher benchmark. Run: `cargo run --release -p spike-mesher`.

use std::hint::black_box;
use std::time::{Duration, Instant};

use spike_mesher::{Mesh, OpacityCols, mesh, scenes, t_junctions};

const RUNS: usize = 3000;

fn main() {
    let cases: Vec<(&str, Vec<u16>)> = vec![
        ("terrain (rough)", scenes::terrain()),
        ("terrain (smooth)", scenes::terrain_smooth()),
        ("flat", scenes::flat(16)),
        ("boxes", scenes::boxes(7, 40)),
        ("solid", scenes::solid()),
        ("empty", scenes::empty()),
        ("noise 50%, 4 mats", scenes::noise(1, 50, 4)),
        ("checker (worst)", scenes::checker()),
    ];

    println!(
        "{:<20} {:>10} {:>10} {:>10} {:>8} {:>10} {:>7}",
        "case", "median", "p99", "masks", "quads", "naive", "tjunc"
    );
    for (name, voxels) in &cases {
        let cols = OpacityCols::from_voxels(voxels);
        let mut out = Mesh::default();
        let mesh_times = sample(|| {
            mesh(black_box(voxels), black_box(&cols), &mut out);
            black_box(&out);
        });
        let mask_times = sample(|| {
            black_box(OpacityCols::from_voxels(black_box(voxels)));
        });
        mesh(voxels, &cols, &mut out);
        let naive = spike_mesher::naive::faces(voxels).len();
        println!(
            "{name:<20} {:>10} {:>10} {:>10} {:>8} {:>10} {:>7}",
            fmt(percentile(&mesh_times, 50)),
            fmt(percentile(&mesh_times, 99)),
            fmt(percentile(&mask_times, 50)),
            out.quads.len(),
            naive,
            t_junctions(&out),
        );
    }
    println!(
        "\nmedian/p99: greedy mesh with cached opacity masks. masks: building them from voxels."
    );
    println!(
        "naive: unit faces a non-merging mesher would emit. tjunc: coplanar T-junction vertices."
    );
}

fn sample(mut f: impl FnMut()) -> Vec<Duration> {
    for _ in 0..200 {
        f();
    }
    let mut v: Vec<Duration> = (0..RUNS)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect();
    v.sort();
    v
}

fn percentile(sorted: &[Duration], p: usize) -> Duration {
    sorted[(sorted.len() * p / 100).min(sorted.len() - 1)]
}

fn fmt(d: Duration) -> String {
    format!("{:.1} µs", d.as_nanos() as f64 / 1000.0)
}
