//! A small separable image resampler that mirrors the defaults of
//! `stbir_resize_uint8` (stb_image_resize v0.9x): 8-bit linear data, no alpha
//! handling, edge clamping, Catmull-Rom when upsampling and Mitchell when
//! downsampling, with the filter support scaled by the resize ratio.

fn catmull_rom(x: f32) -> f32 {
    let x = x.abs();
    if x < 1.0 {
        1.0 - x * x * (2.5 - 1.5 * x)
    } else if x < 2.0 {
        2.0 - x * (4.0 + x * (0.5 * x - 2.5))
    } else {
        0.0
    }
}

fn mitchell(x: f32) -> f32 {
    let x = x.abs();
    if x < 1.0 {
        (16.0 + x * x * (21.0 * x - 36.0)) / 18.0
    } else if x < 2.0 {
        (32.0 + x * (-60.0 + x * (36.0 - 7.0 * x))) / 18.0
    } else {
        0.0
    }
}

/// Filter taps for one axis: for each output pixel the first input pixel and
/// its normalised weights.
fn contributors(in_size: usize, out_size: usize) -> Vec<(i64, Vec<f32>)> {
    let scale = out_size as f32 / in_size as f32;
    let upsample = scale > 1.0;
    let mut out = Vec::with_capacity(out_size);
    for n in 0..out_size {
        let out_center = n as f32 + 0.5;
        let in_center = out_center / scale;
        // Radius of the filter support in input pixels.
        let radius = if upsample { 2.0 } else { 2.0 / scale };
        let first = (in_center - radius + 0.5).floor() as i64;
        let last = (in_center + radius - 0.5).floor() as i64;
        let mut weights = Vec::with_capacity((last - first + 1).max(0) as usize);
        for i in first..=last {
            let d = (i as f32 + 0.5) - in_center;
            let w = if upsample {
                catmull_rom(d)
            } else {
                mitchell(d * scale)
            };
            weights.push(w);
        }
        let sum: f32 = weights.iter().sum();
        if sum != 0.0 {
            for w in &mut weights {
                *w /= sum;
            }
        }
        out.push((first, weights));
    }
    out
}

/// Equivalent of `stbir_resize_uint8`. Returns `None` on invalid sizes.
pub fn resize_uint8(
    input: &[u8],
    in_w: usize,
    in_h: usize,
    in_stride: usize,
    out_w: usize,
    out_h: usize,
    channels: usize,
) -> Option<Vec<u8>> {
    if in_w == 0 || in_h == 0 || out_w == 0 || out_h == 0 || channels == 0 {
        return None;
    }
    if input.len() < in_stride * (in_h - 1) + in_w * channels {
        return None;
    }

    // Horizontal pass: in_h rows x out_w columns, f32.
    let hx = contributors(in_w, out_w);
    let mut tmp = vec![0f32; in_h * out_w * channels];
    for y in 0..in_h {
        let row = &input[y * in_stride..y * in_stride + in_w * channels];
        for (x, (first, weights)) in hx.iter().enumerate() {
            for c in 0..channels {
                let mut acc = 0f32;
                for (k, w) in weights.iter().enumerate() {
                    let sx = (first + k as i64).clamp(0, in_w as i64 - 1) as usize;
                    acc += f32::from(row[sx * channels + c]) * w;
                }
                tmp[(y * out_w + x) * channels + c] = acc;
            }
        }
    }

    // Vertical pass.
    let vy = contributors(in_h, out_h);
    let mut out = vec![0u8; out_h * out_w * channels];
    for (y, (first, weights)) in vy.iter().enumerate() {
        for x in 0..out_w {
            for c in 0..channels {
                let mut acc = 0f32;
                for (k, w) in weights.iter().enumerate() {
                    let sy = (first + k as i64).clamp(0, in_h as i64 - 1) as usize;
                    acc += tmp[(sy * out_w + x) * channels + c] * w;
                }
                out[(y * out_w + x) * channels + c] = (acc + 0.5).floor().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_image_stays_constant() {
        let src = vec![200u8; 5 * 7];
        for (w, h) in [(3, 4), (10, 14), (5, 7), (1, 1)] {
            let out = resize_uint8(&src, 5, 7, 5, w, h, 1).unwrap();
            assert_eq!(out.len(), w * h);
            assert!(out.iter().all(|&p| (199..=201).contains(&p)));
        }
    }

    #[test]
    fn rejects_empty() {
        assert!(resize_uint8(&[], 0, 0, 0, 1, 1, 1).is_none());
        assert!(resize_uint8(&[0], 1, 1, 1, 0, 1, 1).is_none());
    }

    #[test]
    fn four_channels_are_independent() {
        let mut src = Vec::new();
        for _ in 0..4 {
            src.extend_from_slice(&[10, 20, 30, 40]);
        }
        let out = resize_uint8(&src, 2, 2, 8, 4, 4, 4).unwrap();
        for px in out.chunks(4) {
            assert_eq!(px, &[10, 20, 30, 40]);
        }
    }
}
