use std::path::Path;

use voxel_content::{ContentError, Pack, Registry, RenderLayer};
use voxel_core::{BlockId, Face, Identifier};

fn id(s: &str) -> Identifier {
    Identifier::new(s).unwrap()
}

fn pack(id_: &str, blocks: &[(&str, &str)], textures: &[&str]) -> Result<Pack, ContentError> {
    let blocks: Vec<_> = blocks
        .iter()
        .map(|(p, t)| (p.to_string(), t.to_string()))
        .collect();
    Pack::from_parts(
        &format!("id = \"{id_}\"\nversion = \"1\"\n"),
        &blocks,
        textures.iter().map(|t| t.to_string()).collect(),
    )
}

#[test]
fn base_pack_loads_from_disk() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/base");
    let reg = Registry::build(&[Pack::read_dir(&dir).unwrap()]).unwrap();
    assert_eq!(reg.block_id(&id("base:air")), Some(BlockId::AIR));
    let stone = reg.block_id(&id("base:stone")).unwrap();
    assert!(reg.is_opaque(stone));
    assert!(!reg.is_opaque(BlockId::AIR));
    let glass = reg.block(reg.block_id(&id("base:glass")).unwrap()).unwrap();
    assert_eq!(glass.layer, RenderLayer::Translucent);
    assert!(!glass.opaque);

    let grass = reg.block_id(&id("base:grass_block")).unwrap();
    let top = reg.face_texture(grass, Face::PosY).unwrap();
    let side = reg.face_texture(grass, Face::PosX).unwrap();
    let bottom = reg.face_texture(grass, Face::NegY).unwrap();
    assert_eq!(reg.textures()[top.0 as usize], id("base:grass_top"));
    assert_eq!(reg.textures()[side.0 as usize], id("base:grass_side"));
    assert_eq!(reg.textures()[bottom.0 as usize], id("base:dirt"));
    assert_eq!(reg.face_texture(BlockId::AIR, Face::PosY), None);
}

#[test]
fn ids_are_stable_for_the_same_input() {
    let p = || {
        pack(
            "t",
            &[
                ("b", "[textures]\nall = \"x\"\n"),
                ("a", "[textures]\nall = \"x\"\n"),
                ("air", "air = true\n"),
            ],
            &["x"],
        )
        .unwrap()
    };
    let (r1, r2) = (
        Registry::build(&[p()]).unwrap(),
        Registry::build(&[p()]).unwrap(),
    );
    let names = |r: &Registry| {
        r.blocks()
            .iter()
            .map(|b| b.name.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&r1), ["t:air", "t:a", "t:b"]);
    assert_eq!(names(&r1), names(&r2));
}

#[test]
fn a_second_pack_can_use_the_first_packs_textures() {
    let a = pack("a", &[("air", "air = true\n")], &["rock"]).unwrap();
    let b = pack("b", &[("slab", "[textures]\nall = \"a:rock\"\n")], &[]).unwrap();
    let reg = Registry::build(&[a, b]).unwrap();
    let slab = reg.block_id(&id("b:slab")).unwrap();
    assert!(reg.face_texture(slab, Face::PosY).is_some());
}

fn fails(blocks: &[(&str, &str)], textures: &[&str]) -> ContentError {
    match pack("t", blocks, textures).and_then(|p| Registry::build(&[p])) {
        Err(e) => e,
        Ok(_) => panic!("expected an error"),
    }
}

#[test]
fn bad_packs_are_errors_naming_the_problem() {
    let air = ("air", "air = true\n");
    let ok = ("s", "[textures]\nall = \"x\"\n");

    assert!(matches!(fails(&[ok], &["x"]), ContentError::AirCount(0)));
    assert!(matches!(
        fails(&[air, ("air2", "air = true\n")], &[]),
        ContentError::AirCount(2)
    ));
    assert!(matches!(
        fails(&[air, ("s", "[textures]\nall = \"nope\"\n")], &["x"]),
        ContentError::MissingTexture { .. }
    ));
    assert!(matches!(
        fails(
            &[air, ("s", "layer = \"weird\"\n[textures]\nall = \"x\"\n")],
            &["x"]
        ),
        ContentError::Invalid { .. }
    ));
    assert!(matches!(
        fails(&[air, ("s", "")], &["x"]),
        ContentError::Invalid { .. }
    ));
    assert!(matches!(
        fails(&[air, ("s", "[textures]\ntop = \"x\"\n")], &["x"]),
        ContentError::Invalid { .. }
    ));
    assert!(matches!(
        fails(&[air, ("s", "colour = 3\n")], &[]),
        ContentError::Parse { .. }
    ));
    assert!(matches!(
        fails(&[air, ("S", "air = false\n")], &[]),
        ContentError::Name { .. }
    ));
    assert!(matches!(
        fails(&[("a", "air = true\n[textures]\nall = \"x\"\n")], &["x"]),
        ContentError::Invalid { .. }
    ));
    let big = format!("# {}\n[textures]\nall = \"x\"\n", "x".repeat(70_000));
    assert!(matches!(
        fails(&[air, ("s", &big)], &["x"]),
        ContentError::TooLarge { .. }
    ));
}

#[test]
fn bad_manifests_and_duplicates_are_errors() {
    assert!(matches!(
        Pack::from_parts("id = \"Bad Id\"\nversion = \"1\"\n", &[], vec![]),
        Err(ContentError::Name { .. })
    ));
    assert!(matches!(
        Pack::from_parts("version = \"1\"\n", &[], vec![]),
        Err(ContentError::Parse { .. })
    ));
    let air = pack("t", &[("air", "air = true\n")], &[]).unwrap();
    assert!(matches!(
        Registry::build(&[air.clone(), air]),
        Err(ContentError::AirCount(2))
    ));
    let a = pack(
        "t",
        &[("air", "air = true\n"), ("s", "[textures]\nall = \"x\"\n")],
        &["x"],
    )
    .unwrap();
    let b = pack("t", &[("s", "[textures]\nall = \"x\"\n")], &["x"]).unwrap();
    assert!(matches!(
        Registry::build(&[a, b]),
        Err(ContentError::DuplicateBlock(_))
    ));
}

#[test]
fn missing_pack_directory_is_an_io_error() {
    assert!(matches!(
        Pack::read_dir(Path::new("/definitely/not/here")),
        Err(ContentError::Io { .. })
    ));
}
