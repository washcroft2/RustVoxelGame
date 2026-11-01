//! Phase 0 redb spike (plan sections 8 and 13). Throwaway.
//!
//! Stores synthetic but structurally realistic 32³ chunk blobs (palette-packed
//! blocks, zstd-compressed, optionally with a trained dictionary) and measures
//! bulk writes, point reads, autosave-style overwrites, file growth and
//! compaction. Run: `cargo run --release -p spike-redb`.

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use redb::{Database, ReadableDatabase, ReadableTableMetadata, TableDefinition};

const CHUNKS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("chunks");

/// Chunks per column: 8 underground, 1 surface, 1 air.
const COLUMN_HEIGHT: i32 = 10;
const COLUMNS_SIDE: i32 = 45; // 45 x 45 = 2,025 columns = 20,250 chunks
const COMMIT_BATCH: usize = 500;

fn main() {
    let sparse = std::env::args().any(|a| a == "sparse");
    println!(
        "profile: {}",
        if sparse {
            "sparse ore"
        } else {
            "noisy underground"
        }
    );
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/spike-redb");
    fs::create_dir_all(&dir).expect("create output dir");
    let path = dir.join("world.redb");
    let _ = fs::remove_file(&path);

    // Generate everything up front so generation cost doesn't pollute the timings.
    let mut keys = Vec::new();
    let mut raw = Vec::new();
    for cx in 0..COLUMNS_SIDE {
        for cz in 0..COLUMNS_SIDE {
            for cy in 0..COLUMN_HEIGHT {
                keys.push(key(0, cx, cy, cz));
                raw.push(pack_chunk(cx, cy, cz, sparse));
            }
        }
    }
    let raw_bytes: usize = raw.iter().map(Vec::len).sum();

    let plain: Vec<Vec<u8>> = raw
        .iter()
        .map(|r| zstd::bulk::compress(r, 3).expect("compress"))
        .collect();
    let samples: Vec<&[u8]> = raw.iter().step_by(10).map(Vec::as_slice).collect();
    let dict = zstd::dict::from_samples(&samples, 64 * 1024).expect("train dictionary");
    let mut with_dict_c = zstd::bulk::Compressor::with_dictionary(3, &dict).expect("dict");
    let dicted: Vec<Vec<u8>> = raw
        .iter()
        .map(|r| with_dict_c.compress(r).expect("compress"))
        .collect();

    println!("fs: {} (btrfs on this machine)", path.display());
    println!(
        "chunks: {}  raw packed: {} MB",
        keys.len(),
        raw_bytes / 1_000_000
    );
    print_sizes("zstd level 3", &plain);
    print_sizes("zstd level 3 + 64 KB dictionary", &dicted);
    let surface: Vec<&Vec<u8>> = dicted
        .iter()
        .enumerate()
        .filter(|(i, _)| i % COLUMN_HEIGHT as usize == 8)
        .map(|(_, b)| b)
        .collect();
    println!(
        "  surface chunks only, with dictionary: median {} B",
        median_len(&surface)
    );

    // Store the dictionary-compressed blobs from here on.
    let blobs = dicted;
    let stored_bytes: usize = blobs.iter().map(Vec::len).sum();

    let db = Database::create(&path).expect("create db");
    let io0 = write_bytes();

    // 1. Bulk write in batches, each its own durable commit.
    let mut commit_times = Vec::new();
    let t = Instant::now();
    for batch in (0..keys.len()).step_by(COMMIT_BATCH) {
        let end = (batch + COMMIT_BATCH).min(keys.len());
        let txn = db.begin_write().expect("begin_write");
        {
            let mut table = txn.open_table(CHUNKS).expect("open_table");
            for i in batch..end {
                table
                    .insert(keys[i].as_slice(), blobs[i].as_slice())
                    .expect("insert");
            }
        }
        let c = Instant::now();
        txn.commit().expect("commit");
        commit_times.push(c.elapsed());
    }
    let bulk = t.elapsed();
    commit_times.sort();
    println!(
        "\n== bulk write: {} chunks in batches of {COMMIT_BATCH}",
        keys.len()
    );
    println!(
        "total {:?} = {:.0} chunks/s; commit median {:?} max {:?}",
        bulk,
        keys.len() as f64 / bulk.as_secs_f64(),
        commit_times[commit_times.len() / 2],
        commit_times[commit_times.len() - 1],
    );
    let file = fs::metadata(&path).expect("stat").len() as usize;
    println!(
        "file {} MB for {} MB of blobs ({:.2}x); disk written {} MB",
        file / 1_000_000,
        stored_bytes / 1_000_000,
        file as f64 / stored_bytes as f64,
        (write_bytes() - io0) / 1_000_000
    );

    // 2. Random point reads.
    let order = shuffled(keys.len(), 12345);
    let read = db.begin_read().expect("begin_read");
    let table = read.open_table(CHUNKS).expect("open_table");
    let mut times = Vec::with_capacity(order.len());
    let mut got = 0usize;
    for &i in &order {
        let t = Instant::now();
        let v = table
            .get(keys[i].as_slice())
            .expect("get")
            .expect("present");
        got += v.value().len();
        times.push(t.elapsed());
    }
    assert_eq!(got, stored_bytes, "every blob read back at full length");
    times.sort();
    println!(
        "\n== point reads: {} random keys, one read transaction",
        order.len()
    );
    println!(
        "median {:?}  p99 {:?}  max {:?}",
        times[times.len() / 2],
        times[times.len() * 99 / 100],
        times[times.len() - 1]
    );
    drop(table);
    drop(read);

    // 3. Autosave: overwrite a few hundred random chunks per commit, repeatedly.
    let rounds = 200;
    let per_round = 200;
    let size_before = fs::metadata(&path).expect("stat").len();
    let io1 = write_bytes();
    let mut times = Vec::new();
    let mut rng = 99u64;
    for _ in 0..rounds {
        let txn = db.begin_write().expect("begin_write");
        {
            let mut table = txn.open_table(CHUNKS).expect("open_table");
            for _ in 0..per_round {
                let i = (next(&mut rng) % keys.len() as u64) as usize;
                // A slightly different blob each time, as an edited chunk would be.
                let alt = &blobs[(i + 1) % blobs.len()];
                table
                    .insert(keys[i].as_slice(), alt.as_slice())
                    .expect("insert");
            }
        }
        let t = Instant::now();
        txn.commit().expect("commit");
        times.push(t.elapsed());
    }
    times.sort();
    let written = write_bytes() - io1;
    let logical = rounds * per_round * (stored_bytes / blobs.len());
    println!("\n== autosave: {rounds} commits x {per_round} overwritten chunks");
    println!(
        "commit median {:?}  p99 {:?}  max {:?}",
        times[times.len() / 2],
        times[times.len() * 99 / 100],
        times[times.len() - 1]
    );
    println!(
        "file grew {} MB; wrote {} MB to disk for ~{} MB of changed blobs ({:.1}x amplification)",
        (fs::metadata(&path).expect("stat").len() - size_before) / 1_000_000,
        written / 1_000_000,
        logical / 1_000_000,
        written as f64 / logical as f64
    );

    // 4. Compaction.
    let mut db = db;
    let before = fs::metadata(&path).expect("stat").len();
    let t = Instant::now();
    let compacted = db.compact().expect("compact");
    println!(
        "\n== compact: {} -> {} MB in {:?} (did work: {compacted})",
        before / 1_000_000,
        fs::metadata(&path).expect("stat").len() / 1_000_000,
        t.elapsed()
    );
    let n = db
        .begin_read()
        .expect("begin_read")
        .open_table(CHUNKS)
        .expect("open_table")
        .len()
        .expect("len");
    assert_eq!(n as usize, keys.len(), "no chunks lost");
}

/// Key: dimension u8, then x, y, z as big-endian i32 with the sign bit flipped
/// so byte order matches numeric order and neighbouring chunks sit together.
fn key(dim: u8, x: i32, y: i32, z: i32) -> Vec<u8> {
    let mut k = Vec::with_capacity(13);
    k.push(dim);
    for c in [x, y, z] {
        k.extend_from_slice(&(c as u32 ^ 0x8000_0000).to_be_bytes());
    }
    k
}

/// Palette-packed blocks: `[palette_len u16][palette u16...][bits u8][packed indices]`.
/// Uniform chunks store only the single palette entry. No light or fluid layer.
fn pack_chunk(cx: i32, cy: i32, cz: i32, sparse: bool) -> Vec<u8> {
    let mut blocks = vec![0u16; 32 * 32 * 32];
    let mut rng = (cx as u64) << 40 ^ (cy as u64) << 20 ^ cz as u64 ^ 0x9E37_79B9_7F4A_7C15;
    for z in 0..32 {
        for x in 0..32 {
            let (wx, wz) = (cx * 32 + x as i32, cz * 32 + z as i32);
            let surface_y = 8 * 32
                + (12.0 * (wx as f32 * 0.03).sin()
                    + 9.0 * (wz as f32 * 0.021 + wx as f32 * 0.011).sin()) as i32;
            for y in 0..32 {
                let wy = cy * 32 + y as i32;
                let block = if wy > surface_y {
                    0
                } else if wy == surface_y {
                    2 // grass
                } else if wy + 4 >= surface_y {
                    3 // dirt
                } else {
                    let r = next(&mut rng) % 1000;
                    match r {
                        // Noisy profile: ~1.5% ore, 3% air, 5% gravel, all independent.
                        // Sparse profile: ~0.2% ore and nothing else.
                        0..=14 if !sparse || r < 2 => 10 + (r % 5) as u16,
                        15..=44 if !sparse => 0,
                        45..=94 if !sparse => 6,
                        _ => 1,
                    }
                };
                blocks[(y * 32 + z) * 32 + x] = block;
            }
        }
    }
    encode(&blocks)
}

fn encode(blocks: &[u16]) -> Vec<u8> {
    let mut palette: Vec<u16> = Vec::new();
    for &b in blocks {
        if !palette.contains(&b) {
            palette.push(b);
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(&(palette.len() as u16).to_le_bytes());
    for p in &palette {
        out.extend_from_slice(&p.to_le_bytes());
    }
    if palette.len() == 1 {
        out.push(0);
        return out;
    }
    let bits = match palette.len() {
        2 => 1,
        3..=4 => 2,
        5..=16 => 4,
        17..=256 => 8,
        _ => 16,
    };
    out.push(bits);
    let (mut acc, mut n) = (0u64, 0u32);
    for &b in blocks {
        let index = palette.iter().position(|&p| p == b).expect("in palette") as u64;
        acc |= index << n;
        n += u32::from(bits);
        while n >= 8 {
            out.push(acc as u8);
            acc >>= 8;
            n -= 8;
        }
    }
    if n > 0 {
        out.push(acc as u8);
    }
    out
}

fn next(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

fn shuffled(n: usize, seed: u64) -> Vec<usize> {
    let mut v: Vec<usize> = (0..n).collect();
    let mut s = seed | 1;
    for i in (1..n).rev() {
        v.swap(i, (next(&mut s) % (i as u64 + 1)) as usize);
    }
    v
}

fn print_sizes(label: &str, blobs: &[Vec<u8>]) {
    let mut lens: Vec<usize> = blobs.iter().map(Vec::len).collect();
    lens.sort_unstable();
    println!(
        "{label}: total {} MB, median {} B, p90 {} B, max {} B",
        lens.iter().sum::<usize>() / 1_000_000,
        lens[lens.len() / 2],
        lens[lens.len() * 9 / 10],
        lens[lens.len() - 1]
    );
}

fn median_len(blobs: &[&Vec<u8>]) -> usize {
    let mut v: Vec<usize> = blobs.iter().map(|b| b.len()).collect();
    v.sort_unstable();
    v[v.len() / 2]
}

/// Bytes this process has asked the kernel to write (Linux `/proc/self/io`).
fn write_bytes() -> usize {
    fs::read_to_string("/proc/self/io")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("write_bytes: ")?.trim().parse().ok())
        })
        .unwrap_or(0)
}
