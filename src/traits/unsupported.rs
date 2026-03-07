//! Generic stub for unsupported codec operations.
//!
//! Use [`Unsupported<E>`] as the associated type for decode modes your codec
//! doesn't support, instead of defining custom stub types.

use core::marker::PhantomData;

use crate::{DecodeFrame, ImageInfo, OutputInfo};
use zenpixels::PixelSlice;

use super::decoder::{FrameDecode, StreamingDecode};

/// Stub type for codecs that don't support an operation.
///
/// Use as the associated type for unsupported decode modes:
///
/// ```rust,ignore
/// impl<'a> DecodeJob<'a> for MyDecodeJob<'a> {
///     type Error = At<MyError>;  // or just MyError
///     type Dec = MyDecoder<'a>;
///     type StreamDec = Unsupported<At<MyError>>;
///     type FrameDec = Unsupported<At<MyError>>;
///     // ...
///
///     fn streaming_decoder(self, ..) -> Result<Unsupported<At<MyError>>, At<MyError>> {
///         Err(MyError::from(UnsupportedOperation::RowLevelDecode).start_at())
///     }
///
///     fn frame_decoder(self, ..) -> Result<Unsupported<At<MyError>>, At<MyError>> {
///         Err(MyError::from(UnsupportedOperation::AnimationDecode).start_at())
///     }
/// }
/// ```
///
/// The job's method returns `Err(...)` before an `Unsupported` instance is
/// ever created, so the trait methods below are unreachable in practice.
pub struct Unsupported<E>(PhantomData<fn() -> E>);

impl<E: core::error::Error + Send + Sync + 'static> StreamingDecode for Unsupported<E> {
    type Error = E;

    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, E> {
        unreachable!("Unsupported: streaming decode stub should never be constructed")
    }

    fn info(&self) -> &ImageInfo {
        unreachable!("Unsupported: streaming decode stub should never be constructed")
    }
}

impl<E: core::error::Error + Send + Sync + 'static> FrameDecode for Unsupported<E> {
    type Error = E;

    fn next_frame(&mut self) -> Result<Option<DecodeFrame>, E> {
        unreachable!("Unsupported: frame decode stub should never be constructed")
    }

    fn next_frame_to_sink(
        &mut self,
        _sink: &mut dyn crate::DecodeRowSink,
    ) -> Result<Option<OutputInfo>, E> {
        unreachable!("Unsupported: frame decode stub should never be constructed")
    }
}
