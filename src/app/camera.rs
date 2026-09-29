//! Cameras for any client: a general [`ViewCamera`] (eye, target, up, perspective) that the renderer draws with, plus
//! pointer picking against it, and the first-person [`FpsCamera`] policy `re2` drives, which is just one way to produce a
//! `ViewCamera`. Pure maths (glam only): usable, and tested, in the graphics-free build.
//!
//! Conventions match the rest of the engine: right-handed, Y up, depth 0..1 (the wgpu/DirectX clip range), and a yaw of
//! 0 looks down `-Z` (the same "forward" `sim::player` walks along).

use glam::{Mat4, Vec3, Vec4};

/// A perspective camera for one rendered view. Build it with [`ViewCamera::look_at`], [`ViewCamera::top_down`] or
/// [`FpsCamera::view`], or fill the fields yourself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewCamera {
    /// Where the camera is.
    pub eye: Vec3,
    /// A point it looks at.
    pub target: Vec3,
    /// Which world direction is "up" on screen (must not be parallel to the view direction).
    pub up: Vec3,
    /// Vertical field of view, degrees.
    pub fov_deg: f32,
    /// Near clip distance, m.
    pub near: f32,
    /// Far clip distance, m.
    pub far: f32,
}

impl ViewCamera {
    /// A camera at `eye` looking at `target` with world `+Y` up (60 degree field of view, 0.1..200 m).
    pub fn look_at(eye: Vec3, target: Vec3) -> Self {
        ViewCamera { eye, target, up: Vec3::Y, fov_deg: 60.0, near: 0.1, far: 200.0 }
    }

    /// A camera above `focus` looking down at it, `height` metres up. `tilt_deg` leans it back from straight down
    /// (0 = overhead, 30 = a common "three-quarter" view); `yaw_deg` turns which ground direction is up on screen
    /// (0 = `-Z`, 90 = `+X`: the same yaw convention as player movement, so `forward` input walks up the screen).
    pub fn top_down(focus: Vec3, height: f32, tilt_deg: f32, yaw_deg: f32) -> Self {
        let screen_up = ground_forward(yaw_deg.to_radians());
        let tilt = tilt_deg.clamp(0.0, 80.0).to_radians();
        let eye = focus + Vec3::Y * height - screen_up * (height * tilt.tan());
        ViewCamera { eye, target: focus, up: screen_up, fov_deg: 45.0, near: 0.1, far: (height * 4.0).max(200.0) }
    }

    /// Unit view direction.
    pub fn forward(&self) -> Vec3 {
        (self.target - self.eye).normalize_or(Vec3::NEG_Z)
    }

    /// The world-to-view matrix.
    pub fn view(&self) -> Mat4 {
        glam::camera::rh::view::look_at_mat4(self.eye, self.target, self.up)
    }

    /// The projection for a viewport of `aspect` (width / height).
    pub fn projection(&self, aspect: f32) -> Mat4 {
        glam::camera::rh::proj::directx::perspective(self.fov_deg.to_radians(), aspect.max(1e-4), self.near, self.far)
    }

    /// Projection times view: world to clip space.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }

    /// The world-space ray under pixel `(px, py)` of a `width` x `height` viewport (row 0 at the top): `(origin, unit
    /// direction)`, starting on the near plane.
    pub fn screen_ray(&self, px: f32, py: f32, width: u32, height: u32) -> (Vec3, Vec3) {
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        let ndc_x = 2.0 * px / w - 1.0;
        let ndc_y = 1.0 - 2.0 * py / h;
        let inv = self.view_proj(w / h).inverse();
        let unproject = |z: f32| {
            let p = inv * Vec4::new(ndc_x, ndc_y, z, 1.0);
            p.truncate() / p.w
        };
        let (near, far) = (unproject(0.0), unproject(1.0));
        (near, (far - near).normalize_or(self.forward()))
    }

    /// Where the ray under pixel `(px, py)` meets the horizontal plane at height `plane_y` (`None` when it points away).
    /// Click-to-move and ground placement use this; to hit actual objects, cast [`Self::screen_ray`] with
    /// `red_engine2::hit::raycast_shapes`.
    pub fn pick_ground(&self, px: f32, py: f32, width: u32, height: u32, plane_y: f32) -> Option<Vec3> {
        let (o, d) = self.screen_ray(px, py, width, height);
        if d.y.abs() < 1e-6 {
            return None;
        }
        let t = (plane_y - o.y) / d.y;
        (t >= 0.0).then(|| o + d * t)
    }

    /// The pixel a world point lands on (row 0 at the top), or `None` when it is behind the camera.
    pub fn world_to_screen(&self, p: Vec3, width: u32, height: u32) -> Option<(f32, f32)> {
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        let clip = self.view_proj(w / h) * p.extend(1.0);
        if clip.w <= 1e-6 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(((ndc.x + 1.0) * 0.5 * w, (1.0 - ndc.y) * 0.5 * h))
    }
}

/// The horizontal unit vector for a yaw in radians (0 = `-Z`, turning right toward `+X`).
pub fn ground_forward(yaw: f32) -> Vec3 {
    let (s, c) = yaw.sin_cos();
    Vec3::new(s, 0.0, -c)
}

/// A free-look first-person camera driven by player input rather than a scene keyframe track. Yaw/pitch are
/// radians; yaw 0 / pitch 0 looks down `-Z` (matching the offline engine's default camera convention), yaw increases
/// turning right, pitch increases looking up. One camera *policy*: [`FpsCamera::view`] turns it into the general
/// [`ViewCamera`] the renderer draws with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FpsCamera {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_deg: f32,
    pub near: f32,
    pub far: f32,
}

impl FpsCamera {
    pub const PITCH_LIMIT: f32 = 89.0_f32.to_radians() - 0.001;

    pub fn new(position: Vec3, yaw_deg: f32) -> Self {
        FpsCamera { position, yaw: yaw_deg.to_radians(), pitch: 0.0, fov_deg: 90.0, near: 0.05, far: 200.0 }
    }

    /// Full look direction, pitch included (used for the view matrix).
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sy * cp, sp, -cy * cp).normalize()
    }

    /// Horizontal-only look direction (used for walking, so looking up/down doesn't fly you
    /// into the ceiling or floor).
    pub fn forward_flat(&self) -> Vec3 {
        ground_forward(self.yaw)
    }

    /// Horizontal-only right vector, perpendicular to `forward_flat`.
    pub fn right_flat(&self) -> Vec3 {
        let f = self.forward_flat();
        Vec3::new(-f.z, 0.0, f.x)
    }

    /// Right vector for the *full* (pitch-included) look direction. Since this viewer never
    /// rolls the camera, it's identical to `right_flat` — exposed under this name for viewmodel
    /// placement, where it naturally pairs with `forward`/`up` rather than the walk-only
    /// `*_flat` vectors.
    pub fn right(&self) -> Vec3 {
        self.right_flat()
    }

    /// Up vector orthogonal to `forward` and `right`, so a held item tips with the player's
    /// pitch (looking down tilts it down) instead of staying screen-locked.
    pub fn up(&self) -> Vec3 {
        self.right().cross(self.forward()).normalize()
    }

    pub fn look(&mut self, dyaw: f32, dpitch: f32) {
        self.yaw += dyaw;
        self.pitch = (self.pitch + dpitch).clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
    }

    /// This first-person view as a general [`ViewCamera`].
    pub fn view(&self) -> ViewCamera {
        ViewCamera { eye: self.position, target: self.position + self.forward(), up: Vec3::Y, fov_deg: self.fov_deg, near: self.near, far: self.far }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_centre_pixel_ray_is_the_view_direction_and_picks_the_focus() {
        let cam = ViewCamera::top_down(Vec3::new(3.0, 0.0, -2.0), 12.0, 25.0, 40.0);
        let (_, d) = cam.screen_ray(400.0, 300.0, 800, 600);
        assert!(d.dot(cam.forward()) > 0.9999, "{d} vs {}", cam.forward());
        let p = cam.pick_ground(400.0, 300.0, 800, 600, 0.0).unwrap();
        assert!(p.distance(Vec3::new(3.0, 0.0, -2.0)) < 1e-3, "{p}");
    }

    #[test]
    fn world_to_screen_and_pick_ground_round_trip() {
        let cam = ViewCamera::top_down(Vec3::ZERO, 10.0, 30.0, 0.0);
        for p in [Vec3::new(2.0, 0.0, 1.0), Vec3::new(-3.0, 0.0, -2.5), Vec3::new(0.5, 0.0, 3.0)] {
            let (x, y) = cam.world_to_screen(p, 1280, 720).unwrap();
            let back = cam.pick_ground(x, y, 1280, 720, 0.0).unwrap();
            assert!(back.distance(p) < 1e-3, "{p} -> ({x}, {y}) -> {back}");
        }
    }

    #[test]
    fn top_down_screen_up_follows_the_movement_yaw() {
        // yaw 0: -Z is up on screen, +X is right, exactly as `forward`/`strafe` input moves the player.
        let cam = ViewCamera::top_down(Vec3::ZERO, 10.0, 0.0, 0.0);
        let (_, up_y) = cam.world_to_screen(Vec3::new(0.0, 0.0, -2.0), 800, 800).unwrap();
        let (right_x, _) = cam.world_to_screen(Vec3::new(2.0, 0.0, 0.0), 800, 800).unwrap();
        assert!(up_y < 400.0 && right_x > 400.0, "up_y {up_y}, right_x {right_x}");
        let turned = ViewCamera::top_down(Vec3::ZERO, 10.0, 0.0, 90.0);
        let (_, y) = turned.world_to_screen(Vec3::new(2.0, 0.0, 0.0), 800, 800).unwrap();
        assert!(y < 400.0, "yaw 90 puts +X at the top: {y}");
    }

    #[test]
    fn the_first_person_view_matches_the_old_matrix() {
        let mut fps = FpsCamera::new(Vec3::new(1.0, 1.6, 2.0), 30.0);
        fps.pitch = 0.2;
        let old = glam::camera::rh::proj::directx::perspective(fps.fov_deg.to_radians(), 1.5, fps.near, fps.far)
            * glam::camera::rh::view::look_at_mat4(fps.position, fps.position + fps.forward(), Vec3::Y);
        assert!(fps.view().view_proj(1.5).abs_diff_eq(old, 1e-6));
    }
}
