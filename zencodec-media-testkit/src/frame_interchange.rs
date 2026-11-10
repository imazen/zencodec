//! Reusable native-frame interchange checks, independent of a codec factory.
use crate::{Conformance, Failure};
use alloc::{string::ToString, vec::Vec};
use zencodec::{
    frame::Frame,
    image::{ImageOutput, Meaning, Row},
};
use zenpixels::{ColorContext, plane::AnyPlane};

fn fail(detail: &str) -> Failure {
    Failure {
        check: "frame interchange",
        detail: detail.into(),
    }
}

/// Compare direct mapping, selected-image access and canonical planar lending.
/// Checks every active sample bit, per-plane extent/stride/encoding/role, current
/// color/alpha, timing and interpretation retention. Padding is not pixel data.
///
/// Run on native decoded outputs, including padded/odd/deep/float cases; a pass
/// establishes agreement across these access paths, not independent decoding
/// correctness or a codec workspace bound. Mapping must be repeatable.
pub fn check_frame_interchange(frame: &Frame) -> Conformance {
    let direct = frame.map().map_err(|e| fail(&e.to_string()))?;
    let mut expected = Vec::new();
    for index in 0..direct.plane_count() {
        let plane = direct
            .plane(index)
            .ok_or_else(|| fail("direct plane disappeared"))?;
        expected.push((
            plane.width(),
            plane.height(),
            plane.stride_bytes().map_err(|e| fail(&e.to_string()))?,
            direct.encoding(index),
            bytes(plane)?,
        ));
    }
    drop(direct);
    frame
        .with_planar(None, |view| -> Conformance {
            let signal = frame.interpretation();
            let unknown = ColorContext::default();
            if view.layout() != &signal.layout
                || view.alpha_mode() != signal.alpha_mode
                || view.color_context() != signal.color_context.as_deref().unwrap_or(&unknown)
                || (view.width(), view.height())
                    != (signal.geometry.coded_width, signal.geometry.coded_height)
                || view.planes().len() != expected.len()
            {
                return Err(fail("lent interpretation changed"));
            }
            for (index, plane) in view.planes().iter().copied().enumerate() {
                let (w, h, stride, encoding, data) = &expected[index];
                if (
                    plane.width(),
                    plane.height(),
                    plane.stride_bytes().map_err(|e| fail(&e.to_string()))?,
                ) != (*w, *h, *stride)
                    || *encoding != Some(view.layout().planes()[index].encoding())
                    || bytes(plane)? != *data
                {
                    return Err(fail("lent samples or encoding changed"));
                }
            }
            Ok(())
        })
        .map_err(|e| fail(&e.to_string()))??;
    let image = ImageOutput::from_frame(frame.clone());
    let interpretation = image.interpretation();
    if interpretation.signal.as_deref() != Some(frame.interpretation().as_ref())
        || interpretation.timing != Some(frame.timing())
        || interpretation.presentation != Some(frame.presentation())
    {
        return Err(fail("image projection lost interpretation or timing"));
    }
    let mapped = image.map().map_err(|e| fail(&e.to_string()))?;
    if mapped.plane_count() != expected.len() {
        return Err(fail("image projection lost a plane"));
    }
    for (index, (w, h, stride, _, data)) in expected.iter().enumerate() {
        let plane = mapped
            .plane(index)
            .map_err(|e| fail(&e.to_string()))?
            .ok_or_else(|| fail("image plane disappeared"))?;
        if plane.meaning
            != &Meaning::Component(frame.interpretation().layout.planes()[index].role())
            || plane.samples.dimensions() != (*w, *h)
            || plane.samples.stride_bytes() != *stride
        {
            return Err(fail("image projection changed role or geometry"));
        }
        let mut actual = Vec::new();
        for y in 0..*h {
            match plane
                .samples
                .row(y)
                .ok_or_else(|| fail("image row disappeared"))?
            {
                Row::Bytes(row) => actual.extend_from_slice(row),
                Row::Words16(row) => row
                    .iter()
                    .for_each(|v| actual.extend_from_slice(&v.to_le_bytes())),
                Row::Floats(row) => row
                    .iter()
                    .for_each(|v| actual.extend_from_slice(&v.to_bits().to_le_bytes())),
                _ => return Err(fail("unqualified image row type")),
            }
        }
        if actual != *data {
            return Err(fail("image projection changed active sample bits"));
        }
    }
    Ok(())
}
fn bytes(plane: AnyPlane<'_>) -> Result<Vec<u8>, Failure> {
    let mut out = Vec::new();
    match plane {
        AnyPlane::U8(p) => p.rows().for_each(|row| out.extend_from_slice(row)),
        AnyPlane::U16(p) => p.rows().for_each(|row| {
            row.iter()
                .for_each(|v| out.extend_from_slice(&v.to_le_bytes()))
        }),
        AnyPlane::F32(p) => p.rows().for_each(|row| {
            row.iter()
                .for_each(|v| out.extend_from_slice(&v.to_bits().to_le_bytes()))
        }),
        _ => return Err(fail("unqualified native plane type")),
    }
    Ok(out)
}
