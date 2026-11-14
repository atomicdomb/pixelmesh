//! Object orientation as Euler angles, in the convention FBX uses by default
//! ("XYZ" order: rotate about X first, then Y, then Z, all about fixed axes).

use glam::{EulerRot, Quat, Vec3};

/// Rotation for Euler angles given in degrees.
pub fn euler_quat(deg: Vec3) -> Quat {
    Quat::from_euler(EulerRot::ZYX, deg.z.to_radians(), deg.y.to_radians(), deg.x.to_radians())
}

/// Euler angles in degrees for a rotation, tidied so tiny errors do not show as -0.0 or 89.99999.
pub fn quat_euler(q: Quat) -> Vec3 {
    let (z, y, x) = q.normalize().to_euler(EulerRot::ZYX);
    let tidy = |r: f32| {
        let d = (r.to_degrees() * 1000.0).round() / 1000.0;
        if d == 0.0 { 0.0 } else { d }
    };
    Vec3::new(tidy(x), tidy(y), tidy(z))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_round_trip_and_order() {
        for deg in [Vec3::new(30.0, 0.0, 0.0), Vec3::new(10.0, 45.0, -70.0), Vec3::new(-120.0, 20.0, 170.0)] {
            let back = quat_euler(euler_quat(deg));
            assert!(euler_quat(back).angle_between(euler_quat(deg)) < 1e-3, "{deg} -> {back}");
        }
        // X is applied first: +Y turned 90 about X points to +Z, then 90 about Z leaves +Z alone.
        let v = euler_quat(Vec3::new(90.0, 0.0, 90.0)) * Vec3::Y;
        assert!(v.distance(Vec3::Z) < 1e-5, "{v}");
    }
}
