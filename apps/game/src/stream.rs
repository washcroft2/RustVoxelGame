//! Chunk streaming for the Phase 1 test world: generate, mesh and upload the chunks around
//! the camera on worker threads.
//!
//! Temporary shape: the client generates terrain itself. From Phase 2 the embedded server
//! generates and the client receives chunks, and this module's generate step goes away.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, unbounded};
use voxel_content::Registry;
use voxel_core::{ChunkKey, ChunkPos, DimensionId};
use voxel_mesh::{ChunkMesh, Mesher, Neighborhood};
use voxel_render::Renderer;
use voxel_world::ChunkData;
use voxel_worldgen::Generator;

#[derive(Copy, Clone, Debug)]
pub struct StreamConfig {
    /// Horizontal radius in chunks around the camera that gets meshed and drawn.
    pub radius: i32,
    /// Vertical chunk range that can hold terrain; chunks outside are air.
    pub min_y: i32,
    pub max_y: i32,
    pub workers: usize,
}

type Neighbours = Box<[Option<Arc<ChunkData>>; 27]>;

enum Job {
    Generate(ChunkKey),
    Mesh(ChunkKey, Neighbours),
}

enum Done {
    Generated(ChunkKey, ChunkData),
    Meshed(ChunkKey, ChunkMesh, f32),
}

#[derive(Copy, Clone, Debug, Default)]
pub struct StreamStats {
    pub loaded: usize,
    pub meshed: usize,
    pub in_flight: usize,
    /// Median of the last mesh jobs, in microseconds.
    pub remesh_median_us: f32,
    pub quads: u64,
}

pub struct Streamer {
    cfg: StreamConfig,
    dim: DimensionId,
    jobs: Sender<Job>,
    done: Receiver<Done>,
    chunks: HashMap<ChunkKey, Arc<ChunkData>>,
    generating: HashSet<ChunkKey>,
    meshing: HashSet<ChunkKey>,
    /// Quad count of each meshed chunk, for the stats.
    meshed: HashMap<ChunkKey, u32>,
    /// Chunk-column offsets around the camera, nearest first.
    offsets: Vec<(i32, i32)>,
    last_center: Option<ChunkPos>,
    dirty: bool,
    recent_mesh_us: Vec<f32>,
    in_flight: usize,
}

impl Streamer {
    pub fn new(
        cfg: StreamConfig,
        dim: DimensionId,
        generator: Generator,
        registry: Arc<Registry>,
    ) -> Self {
        let (jobs, job_rx) = unbounded::<Job>();
        let (done_tx, done) = unbounded::<Done>();
        for _ in 0..cfg.workers.max(1) {
            let (rx, tx) = (job_rx.clone(), done_tx.clone());
            let registry = Arc::clone(&registry);
            thread::spawn(move || worker(&rx, &tx, generator, &registry));
        }
        // Two rings beyond the draw radius are generated: a chunk at the edge of the disc needs
        // all 26 neighbours, and diagonal ones sit up to 1.5 chunks further out.
        let reach = cfg.radius + 2;
        let mut offsets: Vec<(i32, i32)> = (-reach..=reach)
            .flat_map(|dz| (-reach..=reach).map(move |dx| (dx, dz)))
            .filter(|&(dx, dz)| dx * dx + dz * dz <= reach * reach)
            .collect();
        offsets.sort_by_key(|&(dx, dz)| dx * dx + dz * dz);
        Self {
            cfg,
            dim,
            jobs,
            done,
            chunks: HashMap::new(),
            generating: HashSet::new(),
            meshing: HashSet::new(),
            meshed: HashMap::new(),
            offsets,
            last_center: None,
            dirty: true,
            recent_mesh_us: Vec::new(),
            in_flight: 0,
        }
    }

    pub fn stats(&self) -> StreamStats {
        let mut recent = self.recent_mesh_us.clone();
        recent.sort_by(f32::total_cmp);
        StreamStats {
            loaded: self.chunks.len(),
            meshed: self.meshed.len(),
            in_flight: self.in_flight,
            remesh_median_us: recent.get(recent.len() / 2).copied().unwrap_or(0.0),
            quads: self.meshed.values().map(|&q| u64::from(q)).sum(),
        }
    }

    /// True when every chunk inside the draw radius has been meshed and nothing is running.
    pub fn settled(&self) -> bool {
        self.in_flight == 0 && !self.dirty
    }

    fn in_vertical_range(&self, y: i32) -> bool {
        (self.cfg.min_y..=self.cfg.max_y).contains(&y)
    }

    /// Collects finished work and starts new work for a camera in chunk `center`.
    pub fn update(&mut self, center: ChunkPos, renderer: &mut Renderer) {
        while let Ok(d) = self.done.try_recv() {
            self.in_flight -= 1;
            self.dirty = true;
            match d {
                Done::Generated(key, chunk) => {
                    self.generating.remove(&key);
                    self.chunks.insert(key, Arc::new(chunk));
                }
                Done::Meshed(key, mesh, micros) => {
                    self.meshing.remove(&key);
                    // Skip meshes for chunks that were unloaded while the job ran.
                    if self.chunks.contains_key(&key) {
                        self.meshed.insert(key, mesh.quad_count() as u32);
                        renderer.set_chunk(key, &mesh);
                    }
                    self.recent_mesh_us.push(micros);
                    if self.recent_mesh_us.len() > 64 {
                        self.recent_mesh_us.remove(0);
                    }
                }
            }
        }
        if self.last_center != Some(center) {
            self.last_center = Some(center);
            self.dirty = true;
            self.unload_far(center, renderer);
        }
        if !self.dirty {
            return;
        }
        let cap = self.cfg.workers.max(1) * 3;
        let mut scheduled_all = true;
        let offsets = std::mem::take(&mut self.offsets);
        'columns: for &(dx, dz) in &offsets {
            for y in self.cfg.min_y..=self.cfg.max_y {
                if self.in_flight >= cap {
                    scheduled_all = false;
                    break 'columns;
                }
                let key = ChunkKey::new(self.dim, ChunkPos::new(center.x + dx, y, center.z + dz));
                if !self.chunks.contains_key(&key) && !self.generating.contains(&key) {
                    self.generating.insert(key);
                    self.in_flight += 1;
                    let _ = self.jobs.send(Job::Generate(key));
                }
            }
        }
        let r2 = self.cfg.radius * self.cfg.radius;
        'meshes: for &(dx, dz) in &offsets {
            if dx * dx + dz * dz > r2 {
                continue;
            }
            for y in self.cfg.min_y..=self.cfg.max_y {
                let key = ChunkKey::new(self.dim, ChunkPos::new(center.x + dx, y, center.z + dz));
                if self.meshed.contains_key(&key) || self.meshing.contains(&key) {
                    continue;
                }
                let Some(chunk) = self.chunks.get(&key) else {
                    scheduled_all = false;
                    continue;
                };
                if chunk.is_empty() {
                    // Nothing to draw; remember it so it isn't rechecked every update.
                    self.meshed.insert(key, 0);
                    continue;
                }
                if self.in_flight >= cap {
                    scheduled_all = false;
                    break 'meshes;
                }
                let Some(nb) = self.neighbours(key) else {
                    scheduled_all = false;
                    continue;
                };
                self.meshing.insert(key);
                self.in_flight += 1;
                let _ = self.jobs.send(Job::Mesh(key, nb));
            }
        }
        self.offsets = offsets;
        // Stay dirty while anything was left unscheduled, so the next update continues.
        self.dirty = !scheduled_all;
    }

    /// The 27 chunks around `key`, or `None` if a needed neighbour is not generated yet.
    /// Neighbours outside the vertical range are air, as the generator makes them.
    fn neighbours(&self, key: ChunkKey) -> Option<Neighbours> {
        let mut out: Neighbours = Box::new(std::array::from_fn(|_| None));
        for dy in -1..=1 {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let slot = ((dx + 1) + 3 * ((dz + 1) + 3 * (dy + 1))) as usize;
                    let p = ChunkPos::new(key.pos.x + dx, key.pos.y + dy, key.pos.z + dz);
                    if !self.in_vertical_range(p.y) {
                        continue;
                    }
                    out[slot] = Some(Arc::clone(self.chunks.get(&ChunkKey::new(key.dim, p))?));
                }
            }
        }
        Some(out)
    }

    fn unload_far(&mut self, center: ChunkPos, renderer: &mut Renderer) {
        let keep = self.cfg.radius + 4;
        let far = |k: &ChunkKey| {
            let (dx, dz) = (k.pos.x - center.x, k.pos.z - center.z);
            dx * dx + dz * dz > keep * keep
        };
        let gone: Vec<ChunkKey> = self.chunks.keys().filter(|k| far(k)).copied().collect();
        for k in gone {
            self.chunks.remove(&k);
            self.meshed.remove(&k);
            renderer.remove_chunk(k);
        }
    }
}

fn worker(rx: &Receiver<Job>, tx: &Sender<Done>, generator: Generator, registry: &Registry) {
    let mut mesher = Mesher::new();
    let mut mesh = ChunkMesh::default();
    while let Ok(job) = rx.recv() {
        let done = match job {
            Job::Generate(key) => Done::Generated(key, generator.chunk(key, registry)),
            Job::Mesh(key, nb) => {
                // The centre slot is always filled by `neighbours`; an empty answer keeps the
                // in-flight count right if that ever changes.
                let Some(center) = nb[13].as_deref() else {
                    if tx
                        .send(Done::Meshed(key, ChunkMesh::default(), 0.0))
                        .is_err()
                    {
                        return;
                    }
                    continue;
                };
                let mut hood = Neighborhood::new(center);
                for (i, c) in nb.iter().enumerate() {
                    let o = [i as i32 % 3 - 1, i as i32 / 9 - 1, (i as i32 / 3) % 3 - 1];
                    hood.set(o, c.as_deref());
                }
                let t = Instant::now();
                mesher.mesh(&hood, registry, &mut mesh);
                Done::Meshed(key, mesh.clone(), t.elapsed().as_secs_f32() * 1e6)
            }
        };
        if tx.send(done).is_err() {
            return;
        }
    }
}
