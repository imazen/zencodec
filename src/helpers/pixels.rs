//! Bridges between codec declarations and zenpixels' algebraic image spec.

use zenpixels::{
    AlphaMode, ChannelOrder, ColorSpace, ImageSpec, Primaries, SampleLayout, SignalRange, Transfer,
};

/// Declare a physical layout before transfer information is available.
///
/// RGB describes BT.709 primaries and unknown transfer. Gray has no primaries.
/// The layout determines alpha and padding. Other channel families retain
/// their native interpretation where layout alone determines it.
pub const fn codec_spec(layout: SampleLayout) -> ImageSpec {
    let order = layout.order;
    let alpha = if order.has_padding_slot() {
        AlphaMode::Undefined
    } else if order.has_alpha_slot() {
        AlphaMode::Straight
    } else {
        AlphaMode::None
    };
    let color = if order.is_rgb_family() {
        ColorSpace::Rgb {
            primaries: Primaries::Bt709,
            transfer: Transfer::Unknown,
            range: SignalRange::Full,
            alpha,
            white: None,
        }
    } else if order.is_gray_family() {
        ColorSpace::Gray {
            transfer: Transfer::Unknown,
            range: SignalRange::Full,
            alpha,
            white: None,
        }
    } else if order.is_cmyk_family() {
        ColorSpace::Cmyk {
            inverted: matches!(
                order,
                ChannelOrder::CmykInverted | ChannelOrder::CmykaInverted
            ),
            alpha,
            icc: None,
        }
    } else if order.is_oklab_family() {
        ColorSpace::Oklab { alpha }
    } else {
        ColorSpace::Untagged {
            channels: order.channels() as u8,
            alpha,
        }
    };
    ImageSpec::new(layout, color)
}

#[cfg(test)]
pub(crate) fn signal_range(spec: &ImageSpec) -> SignalRange {
    match spec.color {
        ColorSpace::Rgb { range, .. } | ColorSpace::Gray { range, .. } => range,
        _ => SignalRange::Full,
    }
}

pub(crate) fn has_alpha(spec: &ImageSpec) -> bool {
    spec.layout.order.has_alpha_slot()
}

// Copy interpretation into a diagnostic without retaining profile bytes.
// A class-valid ICC remains in the actual operation's spec.
pub(crate) fn redacted_spec(spec: &ImageSpec) -> ImageSpec {
    let mut public = spec.clone();
    match &mut public.color {
        ColorSpace::Icc { alpha, .. } => {
            public.color = ColorSpace::Untagged {
                channels: public.layout.channels() as u8,
                alpha: *alpha,
            };
        }
        ColorSpace::Cmyk { icc, .. } => *icc = None,
        _ => {}
    }
    public
}
