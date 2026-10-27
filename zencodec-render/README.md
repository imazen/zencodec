# zencodec-render

Render a selected zencodec image as an oriented canvas with required color and
pixel format. This companion has no I/O dependency and keeps conversion/CMS
out of the core codec dependency graph.

```rust,ignore
let renderer = Renderer::new(storage).with_cms(&cms);
let image = renderer.render_selected(&mut images, &id)?;
```

The default is full-canvas sRGB RGBA8, with depth rounding and display-range
clipping. Alpha remains present. Use `with_request(RenderRequest::new(descriptor))`
for the current sample color space, then configure the target, region and
conversion options explicitly. Regions use oriented canvas coordinates.
Unknown composition/orientation, unavailable samples, unqualified auxiliary
alignment and unsupported color conversion fail explicitly. A supplied CMS may
be required; omitting it does not authorize relabeling pixels.

`render_selected` uses the collection's cancellation and policy, publishing the
final image once. `render` accepts an already-decoded `ImageOutput`. The managed
storage pool charges output owners, including retained auxiliary planes; it does
not bound native codec/CMS workspace. Rendering currently reconstructs the full
selection and copies geometry even for identity. Planar video, resizing and HDR
display mapping remain unqualified.

This is an unpublished review crate requiring Rust 1.89 and the converter revision
pinned in Cargo.toml. It is isolated from the root workspace while those converter
APIs are unreleased. Applications consuming it by path must use the same root
zenpixels/zenpixels-convert patches; dependency-level patches are not inherited.
No additional zenpixels change was required to move the validated renderer here.

[Contract](../docs/render-contract.md) ·
[Native tests](../perf/spatial-validation/src/canvas_render_tests.rs)
