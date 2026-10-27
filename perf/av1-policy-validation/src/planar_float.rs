//! Private unclipped output for the currently qualified BT.709 matrix route.
//!
//! Normalization/matrix recipe adapted from imazen/zenavif feadc4c94b19,
//! src/yuv_convert.rs::RecipeConsts/convert_one. The final integer scale,
//! saturation and quantization are intentionally absent for float output.
//! Centered bilinear interpolation keeps the native recipe's exact /16 weights.
//! No transfer conversion, HDR rendering or general planar API is implied.
use zencodec::frame::{PlaneView, Subsampling};

#[derive(Clone, Copy)]
pub(super) struct FloatPlan {
    sub: Subsampling,
    y_offset: f32,
    y_scale: f32,
    uv_center: f32,
    uv_scale: f32,
    vr: f32,
    ug: f32,
    vg: f32,
    ub: f32,
}

impl FloatPlan {
    /// Called only after the enclosing source/encoding/siting validation.
    pub(super) fn new(bits: u8, full: bool, sub: Subsampling) -> Self {
        let shift = u32::from(bits) - 8;
        let max = ((1u32 << bits) - 1) as f32;
        let (kr, kb) = (0.2126f32, 0.0722f32);
        let kg = 1.0 - kr - kb;
        Self {
            sub,
            y_offset: if full { 0.0 } else { (16u32 << shift) as f32 },
            y_scale: 1.0 / if full { max } else { (219u32 << shift) as f32 },
            uv_center: (128u32 << shift) as f32,
            uv_scale: 1.0 / if full { max } else { (224u32 << shift) as f32 },
            vr: 2.0 * (1.0 - kr),
            ug: -2.0 * kb * (1.0 - kb) / kg,
            vg: -2.0 * kr * (1.0 - kr) / kg,
            ub: 2.0 * (1.0 - kb),
        }
    }

    /// One active output row. Coordinates refer to the full coded source;
    /// the enclosing operation handles arbitrary destination stride and stop.
    pub(super) fn row(
        &self,
        planes: &[PlaneView<'_>; 3],
        x: usize,
        y: usize,
        out: &mut [[f32; 3]],
    ) {
        for (i, dst) in out.iter_mut().enumerate() {
            let luma = f32::from(planes[0].code(x + i, y).unwrap());
            let u = chroma(&planes[1], x + i, y, self.sub);
            let v = chroma(&planes[2], x + i, y, self.sub);
            let luma = (luma - self.y_offset) * self.y_scale;
            let u = (u - self.uv_center) * self.uv_scale;
            let v = (v - self.uv_center) * self.uv_scale;
            *dst = [
                v.mul_add(self.vr, luma),
                v.mul_add(self.vg, u.mul_add(self.ug, luma)),
                u.mul_add(self.ub, luma),
            ];
        }
    }
}

fn axis(position: usize, length: usize, subsampled: bool) -> (usize, usize, u32) {
    if !subsampled {
        return (position, position, 0);
    }
    let cell = position / 2;
    if position.is_multiple_of(2) {
        (cell.saturating_sub(1), cell, 3)
    } else {
        (cell, (cell + 1).min(length - 1), 1)
    }
}

fn chroma(plane: &PlaneView<'_>, x: usize, y: usize, sub: Subsampling) -> f32 {
    let (x0, x1, wx) = axis(x, plane.width(), sub != Subsampling::Yuv444);
    let (y0, y1, wy) = axis(y, plane.height(), sub == Subsampling::Yuv420);
    let sample = |x, y| u32::from(plane.code(x, y).unwrap());
    // Integer weighted sums preserve exact native interpolation before range
    // normalization, with no crop-relative phase or intermediate quantization.
    let top = (4 - wx) * sample(x0, y0) + wx * sample(x1, y0);
    let bottom = (4 - wx) * sample(x0, y1) + wx * sample(x1, y1);
    ((4 - wy) * top + wy * bottom) as f32 * (1.0 / 16.0)
}
