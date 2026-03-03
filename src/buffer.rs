//! Pixel buffer types — re-exported from [`zenpixels`].
//!
//! All buffer types, pixel format descriptors, and the [`Pixel`] trait
//! are defined in the `zenpixels` crate. This module re-exports them
//! so that downstream code using `zencodec_types::PixelSlice` (etc.)
//! continues to work unchanged.

#[allow(deprecated)]
pub use zenpixels::{
    AlphaMode, Bgrx, BufferError, ChannelLayout, ChannelType, ColorPrimaries, InterleaveFormat,
    Pixel, PixelBuffer, PixelDescriptor, PixelFormat, PixelSlice, PixelSliceMut, Rgbx, SignalRange,
    TransferFunction,
};
