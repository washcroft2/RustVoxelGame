use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use offset_allocator::{Allocation, Allocator};

/// Bytes per quad (three `u32` words).
pub const QUAD_BYTES: u64 = 12;

pub struct Slab {
    pub buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    alloc: Allocator,
}

/// Where a chunk layer's quads live.
#[derive(Copy, Clone)]
pub struct Placement {
    pub slab: usize,
    /// Offset in quads from the start of the slab.
    pub offset: u32,
    pub(crate) allocation: Allocation,
}

struct PendingFree {
    /// The frame count that must complete before this range is reused.
    after: u64,
    slab: usize,
    allocation: Allocation,
}

/// Storage-buffer slabs with a sub-allocator each. A freed range is held back
/// until the GPU has finished every frame submitted before the free, because
/// those frames may still read it.
pub struct QuadPool {
    pub slabs: Vec<Slab>,
    slab_quads: u32,
    submitted: u64,
    completed: Arc<AtomicU64>,
    pending: Vec<PendingFree>,
}

impl QuadPool {
    pub fn new(slab_bytes: u64) -> Self {
        let slab_quads = (slab_bytes / QUAD_BYTES).min(u64::from(u32::MAX)) as u32;
        Self {
            slabs: Vec::new(),
            slab_quads,
            submitted: 0,
            completed: Arc::new(AtomicU64::new(0)),
            pending: Vec::new(),
        }
    }

    /// Finds room for `quads`, adding a slab when none has space. `make` builds a slab's buffer
    /// and bind group. Returns `None` if the request is larger than a whole slab.
    pub fn alloc(
        &mut self,
        quads: u32,
        make: &mut dyn FnMut(u64) -> (wgpu::Buffer, wgpu::BindGroup),
    ) -> Option<Placement> {
        if quads == 0 || quads > self.slab_quads {
            return None;
        }
        for (i, s) in self.slabs.iter_mut().enumerate() {
            if let Some(a) = s.alloc.allocate(quads) {
                return Some(Placement {
                    slab: i,
                    offset: a.offset,
                    allocation: a,
                });
            }
        }
        let (buffer, bind_group) = make(u64::from(self.slab_quads) * QUAD_BYTES);
        let mut alloc = Allocator::new(self.slab_quads);
        let a = alloc.allocate(quads)?;
        self.slabs.push(Slab {
            buffer,
            bind_group,
            alloc,
        });
        Some(Placement {
            slab: self.slabs.len() - 1,
            offset: a.offset,
            allocation: a,
        })
    }

    /// Releases a range once the frames submitted so far have finished on the GPU.
    pub fn free_later(&mut self, p: Placement) {
        self.pending.push(PendingFree {
            after: self.submitted,
            slab: p.slab,
            allocation: p.allocation,
        });
    }

    /// Returns ranges whose frames have completed to their slab.
    pub fn collect(&mut self) {
        let done = self.completed.load(Ordering::Acquire);
        let slabs = &mut self.slabs;
        self.pending.retain(|f| {
            if f.after <= done {
                slabs[f.slab].alloc.free(f.allocation);
                false
            } else {
                true
            }
        });
    }

    /// Call after each queue submission.
    pub fn frame_submitted(&mut self, queue: &wgpu::Queue) {
        self.submitted += 1;
        let (n, done) = (self.submitted, Arc::clone(&self.completed));
        queue.on_submitted_work_done(move || {
            done.fetch_max(n, Ordering::AcqRel);
        });
    }

    /// Quads currently allocated, and quads in ranges waiting to be freed.
    pub fn usage(&self) -> (u64, u64) {
        let total: u64 = self.slabs.iter().map(|_| u64::from(self.slab_quads)).sum();
        let free: u64 = self
            .slabs
            .iter()
            .map(|s| u64::from(s.alloc.storage_report().total_free_space))
            .sum();
        (total - free, self.pending.len() as u64)
    }
}
