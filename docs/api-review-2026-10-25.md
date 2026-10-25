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
| [FFmpeg send/receive](https://www.ffmpeg.org/doxygen/8.1/group__lavc__encdec.html) | Input and output progress are distinct. An EAGAIN on one side excludes EAGAIN on the other; draining is explicit. | Keep media progress deterministic and test the no-progress cases. Application policy should not decide each packet transition. |
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

Production debugging also needs end-to-end cause preservation. A
[native missing-file probe](io-contract-proposal.md#native-error-provenance-2026-10-25)
shows that FileReadAt loses the underlying `std::io::Error` before the policy or
caller boundary sees it. The proposed traced callback alone cannot solve that
gap. Preserve transport causes in the driver error model as well; tracked in
[167](https://github.com/imazen/zencodec/issues/167).

## Independent Argon review and disposition

Gemini 4 Argon High reviewed the pinned source through agy, including the private
resolver, native controls, guide and primary sources above. The review was
read-only: its signature sketches were not compiled. This is the coordinator's
audited synthesis, separating design advice from reproduced defects.

A focused follow-up accepted the late-decision boundary and retracted the
post-consumption job defect, parallel memory-estimate type, mandatory shared
codec error, generic relabeling escape hatch and unmeasured ratios. Its revised
sketches remain uncompiled. In particular, domain typing must retain the existing
opaque operation/question identity; a typed array index alone does not reject
stale answers.

The strongest useful change is to **type the offers within each question and
offer complete routes when choices interact**. The current private `Question`
already distinguishes domains, but its `Choices` all expose an `Alternative`
containing the heterogeneous `Action` enum. A metadata callback must inspect a
second enum to find metadata actions. A domain-specific borrowed view can remove
that second match without introducing another application callback.

Conceptual shape for a private ablation, not current public Rust signatures:

```rust,ignore
enum Question<'a> {
    Metadata {
        facts: &'a MetadataFacts,
        choices: Choices<'a, MetadataOffer>,
    },
    Transcode {
        facts: &'a TranscodeFacts,
        choices: Choices<'a, RouteOffer>,
    },
    Boundary {
        facts: &'a BoundaryFacts,
        choices: Choices<'a, BoundaryOffer>,
    },
    // Other domains retain typed facts and offers.
}
```

Keep the fallible resolver shape and opaque `ChoiceId`. A typed view does not
prove executability: the adapter still validates required effects,
source/configuration identity, resources and commitment state. The ordinary
consumer still supplies one request and an optional preset or resolver.

A transcode route describes the coupled rewrite or decode/convert/encode
operation, output signaling, loss and resource evidence. Avoid asking the caller
to independently select metadata retention, conversion and scheduling, then
discovering that their combination cannot execute. The existing joint-plan
control already establishes the problem with authored costs: 48 selected units
exceed a 40-unit cap although a 32-unit combination fits. It does not measure
codec memory or establish a general search algorithm.

| Recommendation | Disposition and boundary |
|---|---|
| One fallible application resolver, configurable built-in policy and predefined answers | Keep `Result<Decision, At<E>>`, scoped choices, one-shot preset delegation and typed application causes. |
| Domain-specific offers and coherent transcode routes | Test privately with small planners for actual routes; keep Engine/Prepared/receipt assembly out of ordinary callers. |
| Suspend before affected sink effects | Highest production priority, already tracked in [166](https://github.com/imazen/zencodec/issues/166). An ignorable metadata event is insufficient. |
| Remove late-boundary questions from the resolver | Do not adopt. The codec first suspends mechanically; the same application resolver then chooses among permitted responses to new facts. Holding data requires admitted storage. |
| Add calibrated memory estimates as hard bounds | Do not adopt. `ResourceEstimate` already supplies predictions. Calibration, proven upper bounds and enforced accounting are different evidence. Unknown storage is not proof of unbounded storage. |
| Share item/level/region selection across decode paths | Test with the viewport consumer. Keep catalog discovery and temporal order distinct; account for auxiliary-image and gain-map dependencies. A common selector does not promise cheap random access. |
| Require one framework `CodecError` from every codec | Do not adopt. Preserve codec-native errors and trace/erase them without losing causes at the shared boundary. |
| Remove every ignorable setter | Narrow this to required effects. Explicit optimization hints may remain hints. Replace best-effort correctness setters only after shared enforcement works. |

Output timing and retained storage remain independent. Full-image native state
may favor finishing into admitted encoded staging; a qualified sequential path
may need backpressure to bound upstream work. Neither releases a caller-owned
raster. The review adds no measured speed or resident-memory ratio.

### Defect audit

The review found a real default-method mismatch: handwritten `DynEncoder`
implementations inherited `Ok(AT_FINISH)` from `begin`, even when row input was
unsupported. The ordinary `EncoderShim` already delegated to the correctly
rejecting typed default. Fixed in
[`e582270f`](https://github.com/imazen/zencodec/commit/e582270feba86066bdcffbd02f14ec358d8ac9f1):
the direct dyn default returns the concrete `RowLevelEncode` cause before
accepting rows. A failing-before/passing-after regression covers all three paths
and confirms that refusal leaves one-shot encoding available.

Validation in the source export, outside workspace path overrides:

```text
just encoder-begin-check io-contract-check
run-heavy: done rc=0 16s | peak-RSS 0.41GiB | min-avail 17668MiB | peak-load 1.20
```

This ran the targeted regression, workspace all-feature tests and workspace
all-target/all-feature Clippy. The wrapper receipt describes this validation
process, not codec working memory. Public signatures did not change.

Other assertions were qualified instead of being promoted to defects:

- Ordinary post-consumption use of a dyn job is prevented by `into_*` consuming
  `Box<Self>`. The private Option/debug-assert implementation alone does not
  demonstrate a reachable public use-after-consumption bug. Catching a panic
  from a custom consuming setter is a separate recovery question.
- `erase_video_encoder` lacks the decoder's fused wrapper. This is an
  enforcement asymmetry, not evidence of a native encoder violating its
  terminal-error contract. An adversarial backend test should settle the
  desired shared enforcement before adding a public wrapper.
- `DecodePolicy::strict` disables ICC/EXIF extraction. That raises a question
  about interpretation versus extraction, not proof of wrong native pixels.
  Test actual color/orientation behavior before claiming or changing it.
- Pixel metadata and converted pixels can be supplied separately. The source
  documents that responsibility; a stale-metadata counterexample belongs in
  conversion qualification. Relabeling cannot substitute for conversion.
- Private native paths refusing a hard cap with unknown backend storage are
  refusing honestly. This does not describe every codec or every resource limit.

The released comparison remains crates.io **0.1.26**, with root **0.1.27**
unpublished. The review does not change the frozen migration baseline or imply
that every crate in this workspace is unpublished.

### Traced native error lookup

The follow-up suggested a smaller debugging fix than changing every codec's
error type. The existing `find_cause::<E>` walked `source()` links, but `At<E>`
delegates its source to `E::source()`: the original native `E` could be skipped.
The new `find_native_cause_inside_trace_before_and_after_erasure` regression
failed on that lookup before the fix. `find_cause` now also probes `At<E>` at
each link and borrows its stored error. No error is remapped or stringified.

The regression covers the native value and nested unsupported cause, access to
the trace wrapper after boxing, a native leaf with no source, and an absent
cause. Existing category/envelope checks still pass. This does not reconstruct
causes already discarded by FileReadAt/BufferedReadAt; [167](https://github.com/imazen/zencodec/issues/167)
remains open for that separate loss.

```text
just error-cause-check io-contract-check
run-heavy: done rc=0 10s | peak-RSS 0.39GiB | min-avail 17958MiB | peak-load 0.69
```

The same isolated export ran workspace all-feature tests and all-target Clippy.
This is a validation-process receipt, not a codec memory bound. Associated
native error types and public signatures remain unchanged.

### Experiments that decide the remaining shape

| Experiment | Counterexample and acceptance gate |
|---|---|
| Typed offer views | Re-run metadata, execution, media and boundary controls through typed views and the current representation. Require identical effects, refusal causes, choice identity and callback counts; retain stale-choice, preset, erased-error and no_std controls. Compile a callback without a second `Action` match. Measure dispatch cost separately if representation changes. |
| Coherent route offers | Use an exhaustive oracle over the finite authored alternatives in the existing joint-plan control. Offer the feasible route without replaying or undoing a custom choice. Then qualify native JPEG rewrite and decode/convert/encode routes with independent metadata inspection, same-configuration pixel checks, cancellation and a stalled writer. Distinguish bounded search exhaustion from proof of impossibility. |
| Common spatial target | Replay IMI viewport requests through the existing HTTP simulator, then exercise native IMI and TIFF item/level/region reads. Compare pixels to full-image crops at the same level; test cancellation, changed source versions, stale range completions and auxiliary-item dependencies. Measure bytes, requests and allocations against the current adapter before retiring an entry point. |

These are targeted qualification gates, not completed new native experiments.
They complement the production barrier work. A smaller 0.3 should ship checked
requirements/results, trustworthy progress and ownership contracts, and the
existing conveniences. Keep route search, conversion services, transport
scheduling and rich metadata editing in their owning layers.
