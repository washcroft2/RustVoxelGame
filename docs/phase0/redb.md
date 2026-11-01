# Phase 0 spike: redb with chunk blobs

Code: `spikes/redb` (throwaway). Run: `cargo run --release -p spike-redb` (noisy underground)
or `cargo run --release -p spike-redb -- sparse` (sparse ore). redb 4.4.0, zstd 0.14.3 with
only `zdict_builder`.

## Setup

20,250 chunks (45 x 45 columns of 10 chunks: 8 underground, 1 surface, 1 mostly air).
Each is a palette-packed 32³ block array (no light or fluid layer), zstd level 3, with and
without a 64 KB trained dictionary. Key: dimension byte + x, y, z as order-preserving
big-endian `i32` (13 bytes). Value: the blob. One table. Default (immediate) durability.

The block data is synthetic, in two profiles that bound the real thing:
- **noisy**: underground is independent random noise (1.5% ore, 3% air, 5% gravel). Compresses
  badly, so this is a pessimistic blob size.
- **sparse**: 0.2% ore, otherwise stone. Compresses very well, so this is optimistic.

Real chunks (caves, ore veins, structures) will sit between the two.

## Results (AMD Ryzen 7 5700X, NVMe, **btrfs**, release build, warm page cache)

| Measure | Noisy | Sparse |
|---|---|---|
| Median blob, zstd | 4,002 B | 254 B |
| Median blob, zstd + dictionary | 4,056 B | 219 B |
| Total stored, no dict / dict | 65 / 67 MB | 5 / 4 MB |
| Bulk write, 500-chunk durable commits | 46k chunks/s | 92k chunks/s |
| Commit latency, median / max | 7.3 / 11.7 ms | 3.9 / 6.3 ms |
| File size vs blob bytes | 2.0x | 3.4x |
| Random point read, median / p99 | 0.76 / 2.3 µs | 0.47 / 1.1 µs |
| Autosave: 200 commits x 200 overwritten chunks, median / p99 / max | 21 / 104 / 217 ms | 8.6 / 14 / 15 ms |
| Autosave disk writes per commit | 2.9 MB | 1.3 MB |
| Compaction | 134 -> 114 MB, 0.36 s | 16 -> 10 MB, 0.57 s |

After the autosave rounds the file did not grow (freed pages were reused), and compaction kept
every chunk (the spike asserts the count).

## Reading

- redb handles the load without trouble. Commit time is milliseconds to tens of
  milliseconds, so a 5–30 s autosave on the save thread is not a concern, as long as saves stay
  off the tick thread as the architecture says.
- The worst commit seen was 217 ms (noisy profile). That is on btrfs, which is copy-on-write
  and slow on fsync. Windows (NTFS) and macOS (APFS) will differ, so don't read the tail
  numbers as general.
- Write amplification looks large for sparse blobs (26x) because each commit rewrites whole
  4 KB-plus pages for scattered small values. The absolute figure is what matters: about
  1.3–2.9 MB written per 200-chunk commit.
- Reads are served from redb's cache and the page cache in this test. Cold-disk reads were not
  measured.
- The dictionary helped the sparse profile by about 14% at the median (and about 20% in total)
  and did nothing on noise, as expected. This data can't say whether dictionaries are worth it
  for real chunks. That belongs to Phase 3 with real worldgen output.
- Chunk wire size (target ≤2 KB median) can't be judged from this: the two profiles give
  4 KB and 0.25 KB.

## Not covered

Crash consistency under kill -9 (Phase 3 gate), concurrent readers during a write, multiple
dimensions and tables, a region-file fallback comparison, cold reads, and the Windows and Mac
filesystems.
