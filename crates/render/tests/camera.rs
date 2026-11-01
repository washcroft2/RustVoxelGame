//! View-volume maths, with no GPU.

use glam::Vec3;
use voxel_render::{Camera, Frustum};

fn frustum(camera: &Camera) -> Frustum {
    Frustum::from_view_proj(camera.view_proj(16.0 / 9.0))
}

fn chunk_at(c: [f32; 3]) -> bool {
    // The test camera sits at the block origin, so these are camera-relative already.
    let f = frustum(&Camera {
        position: [0.0, 0.0, 0.0],
        far: 500.0,
        ..Camera::default()
    });
    f.intersects_aabb(Vec3::from(c), Vec3::from(c) + Vec3::splat(32.0))
}

#[test]
fn boxes_ahead_are_in_and_boxes_outside_each_plane_are_out() {
    assert!(chunk_at([-16.0, -16.0, -100.0]), "straight ahead");
    assert!(!chunk_at([-16.0, -16.0, 40.0]), "behind (near plane)");
    assert!(!chunk_at([-16.0, -16.0, -600.0]), "beyond the far plane");
    assert!(
        chunk_at([-16.0, -16.0, -480.0]),
        "just inside the far plane"
    );
    assert!(!chunk_at([300.0, -16.0, -100.0]), "far to the right");
    assert!(!chunk_at([-340.0, -16.0, -100.0]), "far to the left");
    assert!(!chunk_at([-16.0, 300.0, -100.0]), "far above");
    assert!(!chunk_at([-16.0, -340.0, -100.0]), "far below");
}

#[test]
fn yaw_turns_the_view() {
    let mut cam = Camera {
        position: [0.0; 3],
        far: 500.0,
        ..Camera::default()
    };
    let target = (
        Vec3::new(-100.0, -16.0, -16.0),
        Vec3::new(-68.0, 16.0, 16.0),
    );
    // Positive yaw turns left (toward -X), so a box at -X comes into view.
    assert!(!frustum(&cam).intersects_aabb(target.0, target.1));
    cam.yaw = 90f32.to_radians();
    assert!(frustum(&cam).intersects_aabb(target.0, target.1));
}

#[test]
fn camera_origin_is_exact_far_from_zero() {
    let cam = Camera {
        position: [1_000_000_000.25, 80.5, -2_000_000_000.75],
        ..Camera::default()
    };
    assert_eq!(cam.block_origin(), [1_000_000_000, 80, -2_000_000_001]);
    let eye = cam.eye_in_block();
    assert!(
        (eye.x - 0.25).abs() < 1e-6 && (eye.y - 0.5).abs() < 1e-6 && (eye.z - 0.25).abs() < 1e-6
    );
}
