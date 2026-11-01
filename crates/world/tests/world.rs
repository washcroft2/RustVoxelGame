use proptest::prelude::*;
use std::sync::Arc;
use voxel_core::{BlockClass, BlockId, CHUNK_VOLUME, FluidId, LocalPos, TANGENT_U, TANGENT_V};
use voxel_world::{ChunkData, PalettedArray};

/// Test classification: 0 is air, then ids cycle opaque, cutout, translucent.
fn class(id: BlockId) -> BlockClass {
    match id.0 {
        0 => BlockClass::Air,
        n => [
            BlockClass::Opaque,
            BlockClass::Cutout,
            BlockClass::Translucent,
        ][(n as usize - 1) % 3],
    }
}

/// Writes are (index, value). Values are drawn from a small or a wide range
/// so the sequences cross every width from uniform to 16-bit.
fn ops(max_value: u16) -> impl Strategy<Value = Vec<(usize, u16)>> {
    prop::collection::vec((0..CHUNK_VOLUME, 0..=max_value), 0..400)
}

fn check_against_model(base: u16, ops: &[(usize, u16)]) {
    let mut model = vec![base; CHUNK_VOLUME];
    let mut arr = PalettedArray::uniform(base);
    for &(i, v) in ops {
        assert_eq!(arr.set(i, v), model[i]);
        model[i] = v;
    }
    assert_eq!(arr.to_vec(), model);
    arr.compact();
    assert_eq!(arr.to_vec(), model);
    let rebuilt = PalettedArray::from_slice(&model).unwrap();
    assert_eq!(rebuilt, arr);
    assert_eq!(rebuilt.bits(), arr.bits());
}

proptest! {
    #[test]
    fn matches_a_plain_vec_with_few_values(o in ops(3)) { check_against_model(0, &o) }

    #[test]
    fn matches_a_plain_vec_with_some_values(o in ops(40)) { check_against_model(0, &o) }

    #[test]
    fn matches_a_plain_vec_with_many_values(o in ops(u16::MAX)) { check_against_model(7, &o) }

    #[test]
    fn dense_writes_cross_every_width(vals in prop::collection::vec(0u16..1000, CHUNK_VOLUME)) {
        let arr = PalettedArray::from_slice(&vals).unwrap();
        prop_assert_eq!(arr.to_vec(), vals);
    }
}

#[test]
fn payload_sizes_match_the_plan() {
    let sized = |distinct: usize| {
        let vals: Vec<u16> = (0..CHUNK_VOLUME).map(|i| (i % distinct) as u16).collect();
        let a = PalettedArray::from_slice(&vals).unwrap();
        (a.bits(), a.payload_bytes())
    };
    assert_eq!(sized(1), (0, 0));
    assert_eq!(sized(2), (1, 4096));
    assert_eq!(sized(3), (2, 8192));
    assert_eq!(sized(4), (2, 8192));
    assert_eq!(sized(5), (4, 16384));
    assert_eq!(sized(16), (4, 16384));
    assert_eq!(sized(17), (8, 32768));
    assert_eq!(sized(256), (8, 32768));
    assert_eq!(sized(257), (16, 65536));
}

#[test]
fn writes_grow_one_width_at_a_time() {
    let mut a = PalettedArray::uniform(0);
    let mut seen = vec![a.bits()];
    for v in 1..300u16 {
        a.set(v as usize, v);
        if *seen.last().unwrap() != a.bits() {
            seen.push(a.bits());
        }
    }
    assert_eq!(seen, [0, 1, 2, 4, 8, 16]);
    for v in 1..300u16 {
        assert_eq!(a.get(v as usize), v);
    }
}

#[test]
fn out_of_range_access_is_harmless() {
    let mut a = PalettedArray::uniform(5);
    assert_eq!(a.get(CHUNK_VOLUME), 0);
    assert_eq!(a.set(CHUNK_VOLUME, 9), 0);
    assert!(a.is_uniform());
    assert!(PalettedArray::from_slice(&[0; 10]).is_none());
}

#[test]
fn compact_returns_a_drifted_array_to_uniform() {
    let mut a = PalettedArray::uniform(1);
    a.set(10, 2);
    a.set(10, 1);
    assert_eq!(a.bits(), 1);
    a.compact();
    assert_eq!(a.uniform_value(), Some(1));
}

/// Recomputes everything the chunk caches, straight from its blocks.
fn check_cached_state(c: &ChunkData) {
    let blocks = c.blocks().to_vec();
    let count = |k: BlockClass| blocks.iter().filter(|&&b| class(BlockId(b)) == k).count();
    let s = c.summary();
    assert_eq!(s.opaque as usize, count(BlockClass::Opaque));
    assert_eq!(s.cutout as usize, count(BlockClass::Cutout));
    assert_eq!(s.translucent as usize, count(BlockClass::Translucent));
    assert_eq!(
        s.non_air as usize,
        blocks.iter().filter(|&&b| b != 0).count()
    );
    for axis in 0..3 {
        for v in 0..32 {
            for u in 0..32 {
                let mut want = 0u32;
                for d in 0..32 {
                    let mut p = [0u32; 3];
                    p[axis] = d;
                    p[TANGENT_U[axis]] = u as u32;
                    p[TANGENT_V[axis]] = v as u32;
                    let id = BlockId(c.blocks().get(LocalPos::new(p[0], p[1], p[2]).index()));
                    want |= u32::from(class(id) == BlockClass::Opaque) << d;
                }
                assert_eq!(c.opaque_column(axis, u, v), want, "axis {axis} u {u} v {v}");
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]

    #[test]
    fn cached_state_matches_a_recount(
        start in 0u16..4,
        edits in prop::collection::vec((0..CHUNK_VOLUME, 0u16..8, 0u16..3), 0..300),
    ) {
        let mut c = ChunkData::filled(BlockId(start), &class);
        check_cached_state(&c);
        for (i, b, f) in edits {
            let p = LocalPos::from_index(i).unwrap();
            c.set_block(p, BlockId(b), &class);
            c.set_fluid(p, FluidId(f));
        }
        check_cached_state(&c);
        let fluid = c.fluid_layer().map_or(0, |f| f.to_vec().iter().filter(|&&x| x != 0).count());
        prop_assert_eq!(c.summary().fluid_cells as usize, fluid);
        prop_assert_eq!(c.fluid_layer().is_some(), fluid > 0);
        // Rebuilding from the blocks gives the same cached state.
        let rebuilt = ChunkData::from_blocks(c.blocks().clone(), &class);
        prop_assert_eq!(rebuilt.summary().opaque, c.summary().opaque);
        check_cached_state(&rebuilt);
    }
}

#[test]
fn opaque_chunk_edited_back_to_uniform_drops_its_masks() {
    let mut c = ChunkData::filled(BlockId(1), &class);
    assert_eq!(c.opaque_column(1, 3, 4), u32::MAX);
    let p = LocalPos::new(3, 7, 4);
    c.set_block(p, BlockId::AIR, &class);
    check_cached_state(&c);
    c.set_block(p, BlockId(1), &class);
    check_cached_state(&c);
    assert_eq!(c.opaque_column(1, 3, 4), u32::MAX);
}

#[test]
fn version_bumps_only_on_change() {
    let mut c = ChunkData::empty();
    let p = LocalPos::new(1, 2, 3);
    assert_eq!(c.version(), 0);
    c.set_block(p, BlockId::AIR, &class);
    assert_eq!(c.version(), 0);
    c.set_block(p, BlockId(4), &class);
    assert_eq!(c.version(), 1);
    c.set_block(p, BlockId(4), &class);
    assert_eq!(c.version(), 1);
    c.set_fluid(p, FluidId::NONE);
    assert_eq!(c.version(), 1);
    c.set_fluid(p, FluidId(1));
    assert_eq!(c.version(), 2);
    c.compact();
    assert_eq!(c.version(), 2);
}

#[test]
fn copy_on_write_leaves_snapshots_alone() {
    let mut shared = Arc::new(ChunkData::empty());
    let snapshot = Arc::clone(&shared);
    let p = LocalPos::new(0, 0, 0);
    Arc::make_mut(&mut shared).set_block(p, BlockId(2), &class);
    assert_eq!(snapshot.block(p), BlockId::AIR);
    assert_eq!(shared.block(p), BlockId(2));
    // No snapshot outstanding now, so this write does not copy.
    let before = Arc::as_ptr(&shared);
    Arc::make_mut(&mut shared).set_block(p, BlockId(3), &class);
    assert_eq!(Arc::as_ptr(&shared), before);
}

#[test]
fn filled_and_from_blocks_count_correctly() {
    assert!(ChunkData::empty().is_empty());
    assert_eq!(
        ChunkData::filled(BlockId(1), &class).summary().non_air as usize,
        CHUNK_VOLUME
    );
    let mut a = PalettedArray::uniform(0);
    a.set(5, 3);
    a.set(6, 3);
    let c = ChunkData::from_blocks(a, &class);
    assert_eq!((c.summary().non_air, c.summary().translucent), (2, 2));
}
