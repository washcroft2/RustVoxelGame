use proptest::prelude::*;
use voxel_core::*;

proptest! {
    #[test]
    fn block_pos_splits_and_recombines(x: i32, y: i32, z: i32) {
        let p = BlockPos::new(x, y, z);
        prop_assert_eq!(p.chunk().block(p.local()), p);
    }

    #[test]
    fn chunk_contains_its_blocks(cx in -1000i32..1000, cy in -1000i32..1000, cz in -1000i32..1000, i in 0usize..CHUNK_VOLUME) {
        let c = ChunkPos::new(cx, cy, cz);
        let l = LocalPos::from_index(i).unwrap();
        let b = c.block(l);
        prop_assert_eq!(b.chunk(), c);
        prop_assert_eq!(b.local(), l);
        prop_assert_eq!(l.index(), (l.x() | l.z() << 5 | l.y() << 10) as usize);
    }

    #[test]
    fn offset_then_opposite_returns(x in -1_000_000i32..1_000_000, y in -5000i32..5000, z in -1_000_000i32..1_000_000, f in 0usize..6) {
        let p = BlockPos::new(x, y, z);
        let f = Face::from_index(f).unwrap();
        prop_assert_eq!(p.offset(f).offset(f.opposite()), p);
    }

    #[test]
    fn extreme_positions_do_not_panic(x: i32, y: i32, z: i32) {
        for f in Face::ALL {
            let _ = BlockPos::new(x, y, z).offset(f);
            let _ = BlockPos::new(x, y, z).chunk().neighbor(f).origin();
        }
    }
}

#[test]
fn negative_blocks_round_down() {
    assert_eq!(
        BlockPos::new(-1, -32, -33).chunk(),
        ChunkPos::new(-1, -1, -2)
    );
    assert_eq!(BlockPos::new(-1, 0, 0).local(), LocalPos::new(31, 0, 0));
}

#[test]
fn local_index_range() {
    assert!(LocalPos::from_index(CHUNK_VOLUME).is_none());
    assert_eq!(LocalPos::new(31, 31, 31).index(), CHUNK_VOLUME - 1);
    // Out-of-range coordinates wrap instead of escaping the chunk.
    assert_eq!(LocalPos::new(32, 0, 0), LocalPos::new(0, 0, 0));
}

#[test]
fn faces_are_consistent() {
    for (i, f) in Face::ALL.into_iter().enumerate() {
        assert_eq!(f.index(), i);
        assert_eq!(f.opposite().opposite(), f);
        assert_eq!(f.opposite().normal().map(|n| -n), f.normal());
        assert_eq!(f.normal()[f.axis()], f.sign());
    }
    assert_eq!(Face::from_index(6), None);
}

#[test]
fn identifiers() {
    let id = Identifier::new("base:stone/smooth").unwrap();
    assert_eq!((id.namespace(), id.path()), ("base", "stone/smooth"));
    assert_eq!(id.to_string(), "base:stone/smooth");
    assert_eq!(
        Identifier::new("stone"),
        Err(IdentifierError::MissingNamespace)
    );
    assert_eq!(Identifier::new(":stone"), Err(IdentifierError::EmptyPart));
    assert_eq!(Identifier::new("base:"), Err(IdentifierError::EmptyPart));
    assert_eq!(
        Identifier::new("Base:stone"),
        Err(IdentifierError::BadChar('B'))
    );
    assert_eq!(
        Identifier::new("a/b:stone"),
        Err(IdentifierError::BadChar('/'))
    );
    assert_eq!(Identifier::new(""), Err(IdentifierError::BadLength));
    assert_eq!(
        Identifier::new(&format!("a:{}", "x".repeat(200))),
        Err(IdentifierError::BadLength)
    );
}
