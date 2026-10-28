//! Checked display rendering for shared zencodec image collections.
//!
//! Rendering applies canvas orientation and an optional display-coordinate crop,
//! requires the selected packed color output, and preserves qualified aligned
//! auxiliary planes. Unknown interpretation and unsupported composition fail
//! explicitly. Native decoding/CMS workspace and proportional region decoding
//! are not covered by the managed output budget.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
pub mod geometry;
mod render;
pub use render::{ColorTarget, RenderFailure, RenderRequest, RenderStage, Renderer};

/// Render an existing selection with default sRGB RGBA8 output.
/// The collection's stored policy runs once on the final rendered image.
///
/// ```
/// use zencodec::{collection::{Collection, ImageId},
///     operation::{CheckedCollection, OperationPolicy}, storage::StorageBudget};
/// use zencodec_render::Renderer;
/// fn render<C: Collection, R: OperationPolicy>(
///     images: &mut CheckedCollection<C, R>, id: &ImageId,
/// ) -> Result<(), Box<dyn core::error::Error>> {
///     let renderer = Renderer::new(StorageBudget::new(64 * 1024 * 1024, 32));
///     if let Some(first) = renderer.render_first(images)? {
///         first.with_pixels(|pixels| assert_eq!(pixels.descriptor(), zenpixels::PixelDescriptor::RGBA8_SRGB))?;
///     }
///     let image = renderer.render_selected(images, id)?;
///     image.with_pixels(|pixels| assert_eq!(pixels.descriptor(), zenpixels::PixelDescriptor::RGBA8_SRGB))?;
///     Ok(())
/// }
/// ```
#[cfg(doctest)]
struct RenderExample;
