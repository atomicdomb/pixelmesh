//! Orbit camera for the 3D viewport.

use eframe::egui::{Pos2, Rect};
use glam::{Mat4, Vec2, Vec3};

pub struct Camera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub ortho: bool,
}

const FOV_Y: f32 = 0.7;

impl Default for Camera {
    fn default() -> Self {
        Camera { target: Vec3::new(0.0, 0.5, 0.0), yaw: 0.6, pitch: 0.45, distance: 5.0, ortho: false }
    }
}

impl Camera {
    /// Unit vector from the target towards the eye.
    pub fn back(&self) -> Vec3 {
        Vec3::new(self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), self.yaw.cos() * self.pitch.cos())
    }

    pub fn eye(&self) -> Vec3 {
        self.target + self.back() * self.distance
    }

    pub fn view(&self) -> Mat4 {
        // Looking straight down/up makes Y a degenerate "up"; fall back to the yaw direction.
        let up = if self.pitch.abs() > 1.55 {
            Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos()) * self.pitch.signum()
        } else {
            Vec3::Y
        };
        Mat4::look_at_rh(self.eye(), self.target, up)
    }

    pub fn proj(&self, aspect: f32) -> Mat4 {
        if self.ortho {
            let h = self.distance * (FOV_Y * 0.5).tan();
            Mat4::orthographic_rh_gl(-h * aspect, h * aspect, -h, h, -500.0, 500.0)
        } else {
            Mat4::perspective_rh_gl(FOV_Y, aspect, 0.05, 500.0)
        }
    }

    pub fn view_proj(&self, rect: Rect) -> Mat4 {
        self.proj(rect.width() / rect.height().max(1.0)) * self.view()
    }

    /// World-space ray through a screen position: (origin, direction).
    pub fn ray(&self, rect: Rect, pos: Pos2) -> (Vec3, Vec3) {
        let ndc = Vec2::new(
            (pos.x - rect.left()) / rect.width() * 2.0 - 1.0,
            1.0 - (pos.y - rect.top()) / rect.height() * 2.0,
        );
        let inv = self.view_proj(rect).inverse();
        let near = inv.project_point3(Vec3::new(ndc.x, ndc.y, -1.0));
        let far = inv.project_point3(Vec3::new(ndc.x, ndc.y, 1.0));
        (near, (far - near).normalize())
    }

    /// Screen position of a world point, or None when it is behind the camera.
    pub fn project(&self, rect: Rect, p: Vec3) -> Option<Pos2> {
        let clip = self.view_proj(rect) * p.extend(1.0);
        if clip.w <= 1e-6 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Pos2::new(
            rect.left() + (ndc.x * 0.5 + 0.5) * rect.width(),
            rect.top() + (0.5 - ndc.y * 0.5) * rect.height(),
        ))
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(-1.5707, 1.5707);
    }

    pub fn pan(&mut self, dx: f32, dy: f32, viewport_height: f32) {
        let units_per_px = 2.0 * self.distance * (FOV_Y * 0.5).tan() / viewport_height.max(1.0);
        let inv = self.view().inverse();
        let right = inv.transform_vector3(Vec3::X);
        let up = inv.transform_vector3(Vec3::Y);
        self.target += (-right * dx + up * dy) * units_per_px;
    }

    pub fn zoom(&mut self, factor: f32) {
        self.distance = (self.distance / factor).clamp(0.2, 200.0);
    }
}
