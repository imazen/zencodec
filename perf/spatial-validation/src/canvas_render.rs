//! Native tests execute the shared renderer.
use std::sync::Arc;
use zencodec::{
    At, CodecError,
    collection::{Cause, Collection},
    image::{ImageOutput, Rect},
    operation::{CheckedCollection, PolicySet},
    storage::{StorageBudget, StorageError},
};
use zencodec_render::{
    ColorTarget as Target, RenderFailure as Failure, RenderRequest as Request,
    RenderStage as Stage, Renderer,
};
use zenpixels::{Cicp, PixelDescriptor};
use zenpixels_convert::ConvertOptions;
#[path = "canvas_render_tests.rs"]
pub(crate) mod tests;
