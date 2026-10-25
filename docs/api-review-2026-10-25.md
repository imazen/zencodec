# API review against established codec interfaces

Review baseline: main `c949d3f598a84c97ae7b5085ab7ebdf763daa366`, 2026-10-25.
The private accounting-scope regression found during this review is fixed in
`d1de6c1cd0ee64c69f9c668938938eef2c1ff5d6`. This document reviews the
[0.3 proposal](minimal-0.3.md); it does not declare new public signatures stable.

## What remains unproven

Production codecs do not yet implement the proposed mandatory interpretation
barrier. Native whole-operation storage accounting, video epoch transitions
through conversion, and a viewport adapter remain qualification work. The
resolver experiments and guide reader study do not establish those properties.
The reader study also does not measure how difficult a codec implementation is.

The strongest direction is a small codec protocol with a checked application
boundary. One application resolver can cover many situational choices without
making every codec know about transport, privacy, conversion and scheduling.
Keep the resolver, but make the adapter responsible for constructing executable
alternatives and enforcing the selected contract.

## Useful precedents

These are property comparisons, not a ranking or an assertion that another
library satisfies zencodec's metadata/privacy requirements. Sources were read
on 2026-10-25; WebCodecs is a working draft.

| Interface | Verified property | Consequence for zencodec |
|---|---|---|
| [FFmpeg send/receive](https://www.ffmpeg.org/doxygen/8.1/group__lavc__encdec.html) | Input and output progress are distinct. An EAGAIN on one side requires progress on the other; draining is explicit. | Keep media progress deterministic and test the no-progress cases. Application policy should not decide each packet transition. |
| [libjxl decoder](https://libjxl.readthedocs.io/en/latest/api_decoder.html) | Color notification precedes pixels; requesting an output buffer and reporting metadata are distinct events. Subscribed-event completion can precede complete codestream consumption. | A notification, a mandatory pause, final samples and complete input validation need separate meanings. Do not infer whole-file privacy from a color/header event. |
| [Wuffs statuses](https://github.com/google/wuffs/blob/main/doc/note/statuses.md) | Suspension resumes the same operation; errors disable further status-returning operations on that receiver. | Model resumable needs separately from terminal errors. A sink error is not a policy suspension protocol. |
| [GStreamer buffer pools](https://gstreamer.freedesktop.org/documentation/additional/design/bufferpool.html) | Allocation is negotiated after format selection. Reconfiguration must account for buffers still held from the previous pool. | Budget retained old-epoch frames and new conversion/codec storage together. Reconfiguration cannot relabel existing frames or assume their owners released them. |
| [libvips evaluation](https://www.libvips.org/API/current/how-it-works.html) | Regions request rectangular pixel areas; partial images compute demanded regions. | The gigapixel consumer asks for an image item, level and region. HTTP range scheduling is an adapter concern underneath that request. |
| [OpenImageIO ImageInput](https://openimageio.readthedocs.io/en/latest/imageinput.html) | Subimages, MIP levels, tile/scanline access and explicit data conversion are independently represented. | A frame sequence and a byte source do not replace page/pyramid selection. Keep sample layout and interpretation explicit. |
| [WebCodecs ownership](https://www.w3.org/TR/webcodecs/#memory-model) | Clones share a media resource, which stays live while referenced; queue size describes pending requests. | Queue depth alone cannot certify a memory bound. Ownership and release determine retained storage. |
| [nostdio 0.2.0](https://docs.rs/nostdio/0.2.0/nostdio/) | Provides synchronous Read/BufRead/Write/Seek with no_std replacements and std re-exports. | Useful for a synchronous parser bridge. It does not replace resumable byte demand or batched network ranges. |

## Smaller boundaries to retain

**Consumer.** Keep CodecSet, config/job conveniences, strided pixel views and
typed results. A request stores required output effects, permitted alternatives
and preferences once. Execution checks that request automatically. Capability
queries remain useful for discovery; successful setters or matching getters are
not proof of achieved effects.

**Adapter.** Own source identity, input/output admission, conversion plans,
metadata services, feasible alternatives and output commitments. The adapter
must carry requirements across all stages. A selected conversion cannot be
admitted separately from an encoder that cannot signal its result. The existing
joint-plan test already demonstrates that refusing one combination does not
prove every combination impossible. Use small domain planners and bounded
coherent alternatives before inventing a general constraint solver.

**Codec implementor.** Own codec facts, native pixel/packet progress and exact
pre-effect suspension points. Keep source/driver/sink roles and the media session
protocol. Codec authors should not implement the application resolver or acquire
dependencies on its disk, HTTP or metadata-editor services.

The existing private callback shape is sufficient to test that separation:

```rust,ignore
trait PolicyResolver {
    type Error: core::error::Error + Send + Sync + 'static;
    fn resolve(&mut self, question: Question<'_>)
        -> Result<Decision, zencodec::At<Self::Error>>;
}
```

This is implemented in the experiment, not exported by zencodec. Preserve its
predefined choose/delegate/decline responses. Do not add an ignore-error response
or call it for every row, allocation or writer stall. A remote decision can be
handled by a driver only after a real suspension contract exists; making every
codec callback async does not supply that contract.

## Improvements that need qualification

1. **Make the boundary before effects the next implementor experiment.** Hold
   conversion, encoder begin and output publication until the relevant facts
   are final for their stated scope. A second step without the required resume
   must not silently advance. Test real late JPEG metadata, a second native
   format and a video epoch. The existing authored QOI discovery control is not
   enough to freeze the production interface.
2. **Bind evidence to each resource's scope.** The disk-only regression shows
   how an unrelated RAM condition can weaken admission. Public callers can
   still select a simple operation budget; internal RAM/disk evidence needs
   independent scope validation. Upper bounds, enforced caps and estimates
   remain different claims. Unknown backend storage must stay unknown.
3. **Use one request path across conveniences and direct jobs.** Check unsupported
   requirements before consuming non-replayable input where facts permit it.
   Keep runtime failures typed. Avoid a separate public prepare/validate/execute
   choreography unless a caller actually needs preparation; ordinary execution
   should invoke the same machinery itself.
4. **Make pixel conversion a first-class pipeline operation.** Use
   zenpixels-convert with source interpretation intact, permitted loss explicit
   and encoder signaling checked. Conversion buffers and scratch participate in
   admission. Row conversion alone does not establish bounded streaming. The
   [native JPEG recipes](pixel-conversion.md) are a narrow tested starting point.
5. **Prove item/level/region selection with the viewport consumer.** Keep it
   separate from temporal frame order. Exercise view cancellation, reused ranges,
   stale completions and immutable source versions using the existing transport
   model, then native item decoding. Do not add another seek hierarchy merely
   to make the call look uniform.

Backpressure follows those same ownership facts. A full-raster encoder may
benefit from admitted encoded staging so it can finish and release its state;
a qualified sequential encoder with pausable upstream may require backpressure.
Finishing never releases a caller-owned raster. No generic streaming preference
can authorize exceeding a cap or invent a lower-memory implementation.

## What to retire only after replacements work

- Best-effort setters for effects presented as requirements, once the checked
  path covers generic, erased and convenience calls.
- Ambiguous preservation promises: distinguish semantic retention, byte-exact
  packets and permissive retention. Opaque data must be dropped only when all
  remaining requirements permit it, or refused.
- Caller-side Engine/Prepared/receipt assembly in ordinary examples. These are
  useful enforcement internals, not a necessary application programming model.
- Millisecond-only implementation requirements after rational timing migration;
  keep exact millisecond conveniences.

Keep separate native ownership protocols where collapsing them would add copies
or buffering. Do not replace the single resolver with many application callbacks,
add a mandatory CMS/editor/runtime dependency to core, or publish a general
planner just because the experiments can express one.

## Evidence from this review

The accounting-scope fix has a failing-before/passing-after test and native
bitmap, disclosure and stream regression checks. Details and the resource-wrapper
receipt are in [policy qualification](policy-scenarios.md#accounting-scope-regression-2026-10-25).
This establishes the selection predicate; it adds no codec memory or throughput
claim. Existing tracking remains [27](https://github.com/imazen/zencodec/issues/27),
[159](https://github.com/imazen/zencodec/issues/159),
[161](https://github.com/imazen/zencodec/issues/161),
[166](https://github.com/imazen/zencodec/issues/166), and
[131](https://github.com/imazen/zencodec/issues/131).
