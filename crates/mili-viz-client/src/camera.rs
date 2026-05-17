//! Orbit camera. Pure math, no GPU — the reusable core every later
//! Phase 5 milestone builds on (M2 mesh display, M4 local
//! manipulation reconciled against the server-authoritative
//! `CameraState`). The field shape (`azimuth`/`elevation`/`distance`
//! plus a focus point) lines up 1:1 with the frozen proto
//! `CameraState` so M4's reconcile step is a field copy, not a
//! conversion
//! (`phase-5-m1.md` Decision 40).

use glam::{Mat4, Vec3};

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
}
