use glam::camera::rh::proj::directx::perspective;
use glam::camera::rh::view::look_at_mat4;
use glam::{Mat4, Vec3, Vec4};

/// A free camera. Position is `f64` so it stays exact far from the origin.
///
/// `yaw` 0 looks along -Z and positive yaw turns left; positive `pitch` looks up.
#[derive(Copy, Clone, Debug)]
pub struct Camera {
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    /// Vertical field of view in radians.
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: [0.0, 80.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 70f32.to_radians(),
            near: 0.1,
            far: 2000.0,
        }
    }
}

impl Camera {
    /// Unit vector the camera looks along.
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(-sy * cp, sp, -cy * cp)
    }

    /// Unit vector to the camera's right, level with the ground.
    pub fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        Vec3::new(cy, 0.0, -sy)
    }

    /// The block the camera is in. Chunk origins are made relative to this
    /// as integers, so far-away worlds don't lose precision.
    pub fn block_origin(&self) -> [i32; 3] {
        self.position.map(|c| c.floor() as i32)
    }

    /// Position inside [`Self::block_origin`], in 0..1.
    pub fn eye_in_block(&self) -> Vec3 {
        let o = self.block_origin();
        Vec3::new(
            (self.position[0] - f64::from(o[0])) as f32,
            (self.position[1] - f64::from(o[1])) as f32,
            (self.position[2] - f64::from(o[2])) as f32,
        )
    }

    /// World-to-clip matrix for coordinates relative to [`Self::block_origin`].
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let eye = self.eye_in_block();
        let view = look_at_mat4(eye, eye + self.forward(), Vec3::Y);
        perspective(self.fov_y, aspect.max(1e-3), self.near, self.far) * view
    }
}

/// The six planes of a view volume, each `(normal, d)` with the inside at `normal . p + d >= 0`.
#[derive(Copy, Clone, Debug)]
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    /// Extracts the planes from a clip matrix with 0..1 depth.
    pub fn from_view_proj(m: Mat4) -> Self {
        let row = |i: usize| Vec4::new(m.x_axis[i], m.y_axis[i], m.z_axis[i], m.w_axis[i]);
        let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
        let norm = |p: Vec4| p / p.truncate().length().max(1e-12);
        Self {
            planes: [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2].map(norm),
        }
    }

    /// False only when the box is entirely outside one plane.
    pub fn intersects_aabb(&self, min: Vec3, max: Vec3) -> bool {
        self.planes.iter().all(|p| {
            // The corner farthest along the plane normal decides.
            let far = Vec3::new(
                if p.x >= 0.0 { max.x } else { min.x },
                if p.y >= 0.0 { max.y } else { min.y },
                if p.z >= 0.0 { max.z } else { min.z },
            );
            p.truncate().dot(far) + p.w >= 0.0
        })
    }
}
