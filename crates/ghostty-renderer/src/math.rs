//! Math helpers (`src/math.zig`), only what the renderer needs.

/// A 4x4 matrix as four column vectors (same layout as Zig's `[4]@Vector(4, f32)`).
pub type Mat = [[f32; 4]; 4];

/// 2D orthographic projection matrix
pub fn ortho2d(left: f32, right: f32, bottom: f32, top: f32) -> Mat {
    let w = right - left;
    let h = top - bottom;
    [
        [2.0 / w, 0.0, 0.0, 0.0],
        [0.0, 2.0 / h, 0.0, 0.0],
        [0.0, 0.0, -1.0, 0.0],
        [-(right + left) / w, -(top + bottom) / h, 0.0, 1.0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ortho2d_maps_corners() {
        let m = ortho2d(0.0, 800.0, 600.0, 0.0);
        let apply = |x: f32, y: f32| {
            (
                m[0][0] * x + m[1][0] * y + m[3][0],
                m[0][1] * x + m[1][1] * y + m[3][1],
            )
        };
        assert_eq!(apply(0.0, 0.0), (-1.0, 1.0));
        assert_eq!(apply(800.0, 600.0), (1.0, -1.0));
    }
}
