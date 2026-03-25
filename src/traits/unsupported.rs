//! Generic stub for unsupported codec operations.
//!
//! Use [`Unsupported<E>`] as the associated type for decode modes your codec
//! doesn't support, instead of defining custom stub types.

use core::marker::PhantomData;

use crate::output::{AnimationFrame, OwnedAnimationFrame};
use crate::sink::SinkError;
use crate::{ImageInfo, OutputInfo};
use enough::Stop;
use zenpixels::PixelSlice;

use super::decoder::{AnimationFrameDecoder, StreamingDecode};

/// Stub type for codecs that don't support an operation.
///
/// Use as the associated type for unsupported decode modes:
///
/// ```rust,ignore
/// impl<'a> DecodeJob<'a> for MyDecodeJob<'a> {
///     type Error = At<MyError>;  // or just MyError
///     type Dec = MyDecoder<'a>;
///     type StreamDec = Unsupported<At<MyError>>;
///     type AnimationFrameDec = Unsupported<At<MyError>>;
///     // ...
///
///     fn streaming_decoder(self, ..) -> Result<Unsupported<At<MyError>>, At<MyError>> {
///         Err(MyError::from(UnsupportedOperation::RowLevelDecode).start_at())
///     }
///
///     fn animation_frame_decoder(self, ..) -> Result<Unsupported<At<MyError>>, At<MyError>> {
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

impl<E: core::error::Error + Send + Sync + 'static> AnimationFrameDecoder for Unsupported<E> {
    type Error = E;

    fn wrap_sink_error(_err: SinkError) -> E {
        unreachable!("Unsupported: full frame decode stub should never be constructed")
    }

    fn info(&self) -> &ImageInfo {
        unreachable!("Unsupported: full frame decode stub should never be constructed")
    }

    fn render_next_frame(
        &mut self,
        _stop: Option<&dyn Stop>,
    ) -> Result<Option<AnimationFrame<'_>>, E> {
        unreachable!("Unsupported: full frame decode stub should never be constructed")
    }

    fn render_next_frame_owned(
        &mut self,
        _stop: Option<&dyn Stop>,
    ) -> Result<Option<OwnedAnimationFrame>, E> {
        unreachable!("Unsupported: full frame decode stub should never be constructed")
    }

    fn render_next_frame_to_sink(
        &mut self,
        _stop: Option<&dyn Stop>,
        _sink: &mut dyn crate::DecodeRowSink,
    ) -> Result<Option<OutputInfo>, E> {
        unreachable!("Unsupported: full frame decode stub should never be constructed")
    }
}
