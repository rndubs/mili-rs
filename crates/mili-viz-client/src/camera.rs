//! Orbit camera. Pure math, no GPU — the reusable core every later
//! Phase 5 milestone builds on (M2 mesh display, M4 local
//! manipulation reconciled against the server-authoritative
//! `CameraState`). The field shape (`azimuth`/`elevation`/`distance`
//! plus a focus point) lines up 1:1 with the frozen proto
//! `CameraState` so M4's reconcile step is a field copy, not a
//! conversion
//! (`phase-5-m1.md` Decision 40).

use glam::{Mat4, Vec3, Vec4};

/// An orbit camera looking at a focus point from a distance, rotated
/// by `azimuth` (about world +Y) and `elevation` (toward world +Y).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Orbit angle about world +Y, radians.
    pub azimuth: f32,
    /// Elevation toward world +Y, radians. Clamped away from the
    /// poles by [`Self::eye`] to keep the up vector well-defined.
    pub elevation: f32,
    /// Eye-to-focus distance, world units. Must be > 0.
    pub distance: f32,
    /// Focus point the camera orbits and looks at.
    pub focus: Vec3,
    /// Vertical field of view, radians.
    pub fov_y: f32,
    /// Near clip plane, world units.
    pub z_near: f32,
    /// Far clip plane, world units.
    pub z_far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            azimuth: 0.0,
            elevation: 0.0,
            distance: 3.0,
            focus: Vec3::ZERO,
            fov_y: 60_f32.to_radians(),
            z_near: 0.1,
            z_far: 100.0,
        }
    }
}

impl Camera {
    /// World-space eye position derived from the orbit parameters.
    #[must_use]
    pub fn eye(&self) -> Vec3 {
        // Keep elevation strictly inside the poles so the +Y up
        // vector never degenerates.
        let limit = std::f32::consts::FRAC_PI_2 - 1e-3;
        let elev = self.elevation.clamp(-limit, limit);
        let (sa, ca) = self.azimuth.sin_cos();
        let (se, ce) = elev.sin_cos();
        let dir = Vec3::new(ce * sa, se, ce * ca);
        self.focus + dir * self.distance.max(f32::MIN_POSITIVE)
    }

    /// Right-handed look-at view matrix.
    #[must_use]
    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), self.focus, Vec3::Y)
    }

    /// Orthonormal camera basis `(right, up, forward)` in world space,
    /// `forward` pointing from the eye toward the focus. Used by M4's
    /// screen-space pan to translate the focus in the view plane
    /// (`phase-5-m4.md` Decision 64).
    #[must_use]
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let forward = (self.focus - self.eye()).normalize_or_zero();
        let mut right = forward.cross(Vec3::Y);
        if right.length_squared() < 1e-12 {
            right = Vec3::X;
        }
        let right = right.normalize();
        let up = right.cross(forward);
        (right, up, forward)
    }

    /// Perspective projection for a `width`/`height` viewport.
    /// `wgpu` clip space has depth in `0..1`, so this uses the
    /// reverse-GL (`_rh`, not `_rh_gl`) variant.
    #[must_use]
    pub fn projection(&self, width: u32, height: u32) -> Mat4 {
        let aspect = width.max(1) as f32 / height.max(1) as f32;
        Mat4::perspective_rh(self.fov_y, aspect, self.z_near, self.z_far)
    }

    /// Combined view-projection matrix the renderer uploads.
    #[must_use]
    pub fn view_projection(&self, width: u32, height: u32) -> Mat4 {
        self.projection(width, height) * self.view()
    }

    /// World-space pick ray `(origin, dir)` (dir normalized) through a
    /// normalized device coordinate (`x,y ∈ -1..1`, y up — the
    /// `wgpu`/glam clip convention). Unprojects the near and far clip
    /// points through `inverse(view_proj)`. Pure math (no GPU), so the
    /// picking test core is always-on.
    #[must_use]
    pub fn ray_from_ndc(&self, ndc_x: f32, ndc_y: f32, width: u32, height: u32) -> (Vec3, Vec3) {
        let inv = self.view_projection(width, height).inverse();
        let unproject = |z: f32| -> Vec3 {
            let p: Vec4 = inv * Vec4::new(ndc_x, ndc_y, z, 1.0);
            p.truncate() / p.w
        };
        // wgpu clip depth is 0 (near) .. 1 (far).
        let near = unproject(0.0);
        let far = unproject(1.0);
        (near, (far - near).normalize_or_zero())
    }

    /// World-space pick ray through a pixel `(px, py)` of a
    /// `width`x`height` viewport, top-left origin (the egui / window
    /// convention). Thin wrapper over [`Self::ray_from_ndc`] that flips
    /// y and maps pixels to NDC.
    #[must_use]
    pub fn ray_from_screen(&self, px: f32, py: f32, width: u32, height: u32) -> (Vec3, Vec3) {
        let w = width.max(1) as f32;
        let h = height.max(1) as f32;
        let ndc_x = 2.0 * px / w - 1.0;
        let ndc_y = 1.0 - 2.0 * py / h;
        self.ray_from_ndc(ndc_x, ndc_y, width, height)
    }

    /// A default-orientation orbit camera framed on a bounding sphere
    /// `(center, radius)` — the M2 auto-frame so a real corpus is in
    /// view regardless of its coordinate scale (`phase-5-m2.md`
    /// Decision 42). Distance puts the sphere comfortably inside the
    /// vertical FOV; the clip planes bracket it.
    #[must_use]
    pub fn looking_at(center: Vec3, radius: f32) -> Self {
        let base = Self::default();
        let r = radius.max(1e-3);
        let distance = r / (base.fov_y * 0.5).sin() * 1.3;
        Self {
            focus: center,
            distance,
            z_near: (distance - r).max(r * 1e-3),
            z_far: distance + r * 4.0,
            ..base
        }
    }

    /// Reconstruct an orbit camera from explicit orbit parameters
    /// (the M4 reconcile core — `phase-5-m4.md` Decision 64). The
    /// server's `CameraState` maps field-for-field onto the first
    /// four args (Decision 40 shaped them 1:1; radians per Decision
    /// 65); `fov_y`/`z_near`/`z_far` are client-only projection params
    /// the proto does not carry, re-bracketed around `distance` and
    /// the cached model `radius` exactly like [`Self::looking_at`].
    /// Proto-free so it is the always-on reconcile test core.
    #[must_use]
    pub fn from_orbit(
        azimuth: f32,
        elevation: f32,
        distance: f32,
        focus: Vec3,
        radius: f32,
    ) -> Self {
        let base = Self::default();
        let r = radius.max(1e-3);
        Self {
            azimuth,
            elevation,
            distance,
            focus,
            z_near: (distance - r).max(r * 1e-3),
            z_far: distance + r * 4.0,
            ..base
        }
    }
}
