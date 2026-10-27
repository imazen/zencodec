//! Native tests execute shared geometry.
use zencodec::{
    CodecError,
    collection::Composition,
    image::{Auxiliary, AuxiliaryGrid, ImageGeometry, ImageOutput, Rect, Samples},
    storage::{StorageBudget, StorageError},
};
pub use zencodec_render::geometry::*;
use zenpixels::{ChannelLayout, Orientation, PixelDescriptor, TransferFunction};
#[path = "canvas_projection_tests.rs"]
mod tests;
