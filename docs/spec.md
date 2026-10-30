> The current review uses `Pulse` / `SharedPulse` for operation control.
> See [the control migration](pulse-migration.md) for caller and codec changes.
> Bare byte requests are `InputRequest`; metadata rewriting uses `RewriteRequest`.

# zencodec API Specification

Shared traits and types for zen* image codecs. This is the canonical reference
for the public API surface.

`#![no_std]` + `alloc`. `#![forbid(unsafe_code)]`.

The [early decode-output commitment](decode-output-commitment.md) is a separate
proposal with native qualification gates; it is not part of the implemented
surface below.

### Operation policy binding (review API)

Pass `PolicySet` directly for built-in selection. Add a callback with
`BoundPolicy::new(requirements, callback)` when a decision depends on the
current offers. Custom callback structs implement `PolicyResolver` and go
inside the binding; closures with no mandatory requirements may be passed
alone. All operation entry points accept the sealed `OperationPolicy` trait.

```rust
let mut policy = BoundPolicy::new(
    PolicySet::new().with_output_wait_limit(8 * 1024 * 1024),
    |question: Question<'_>| -> Result<Decision, At<MyError>> {
        record_question(&question);
        Ok(Decision::UsePreset)
    },
);
```

Admission reads the binding before invoking the callback. Unknown or
contradictory required evidence excludes a route and contributes to the bounded
`DecisionRecord::unmet_requirements`. A callback cannot select an excluded
route. `UsePreset` applies the binding's preferences to the remaining choices.
A direct `PolicySet` selects without callback delegation, so `used_preset`
remains false; this field records delegation, not the origin of every choice.

`with_policy_set` is deprecated. During migration its requirements and the
policy argument's requirements both apply; a bound policy supplies preferences
in place of the legacy setter's preferences. Low-level `resolve_*` adapters
retain their explicit preset parameter for compatibility; new callers use
`PolicySet::new()` there and carry requirements in the policy argument.
`Renderer::render` now takes one policy argument; `render_with_preset` is the
deprecated compatibility form.

Bindings contain configuration and callback state, not pending obligations.
Reusing one across decode and encode does not discharge output requirements at
a decode boundary. Unproven final-effect obligations still refuse. Native
adapter evidence remains the adapter's responsibility.

### Required encode metadata (candidate)

```rust,ignore
let job = config.job()
    .try_with_metadata_policy(metadata, MetadataPolicy::PreserveExact)?;
```

Retain the job on refusal:

```rust,ignore
let mut job = config.job();
job.try_set_metadata_policy(metadata, MetadataPolicy::Web)?;
```

Registry preparation uses the same checked primitive:

```rust,ignore
let request = EncodeRequest::new(format)
    .with_metadata_policy(metadata, MetadataPolicy::PreserveExact);
```

The mutable primitive returns `Result<(), BoxedError>` and refuses atomically.
Typed builders and dyn jobs preserve that cause. Both ordinary and Send request
preparation apply it to retained candidates before selection or pixel input.
Default implementations reject even empty clearing with
`UnsupportedOperation::MetadataRetention`; static capability flags are not
admission. Accepted obligations survive later configuration and must be checked
before affected output. Native retention qualification is still pending; the
legacy best-effort setter is deprecated but remains available during adoption. See
[the native witness and contract](metadata-retention-admission.md).

### zenpixels: use but never re-export

`zenpixels` defines the cross-crate pixel interchange types: `PixelDescriptor`,
`PixelFormat`, `PixelSlice`, `PixelSliceMut`, `PixelBuffer`, `ChannelLayout`,
`ChannelType`, `TransferFunction`, `ColorPrimaries`, `AlphaMode`, `SignalRange`,
`InterleaveFormat`.

**All crates in the zen ecosystem MUST use `zenpixels` types directly.**
zencodec uses them in trait signatures but callers and codec
implementors should depend on `zenpixels` directly and use `zenpixels::` paths
in their public APIs.

---

## Trait hierarchy

```text
ENCODE:
                                 ┌→ Enc (Encoder)
EncoderConfig → EncodeJob ──────┤
                                 └→ AnimationFrameEnc (AnimationFrameEncoder, 'static)

DECODE:
                                 ┌→ Dec (Decode)
DecoderConfig → DecodeJob<'a> ──┤→ StreamDec (StreamingDecode)
                                 └→ AnimationFrameDec (AnimationFrameDecoder, 'static)
```

Each layer has object-safe `Dyn*` variants for codec-agnostic dispatch:

```text
DynEncoderConfig → DynEncodeJob → DynEncoder / DynAnimationFrameEncoder
DynDecoderConfig → DynDecodeJob → DynDecoder / DynStreamingDecoder / DynAnimationFrameDecoder
```

Blanket impls generate the dyn API automatically from the generic traits.

Color management is **not** the codec's job. Decoders return native pixels
with ICC/CICP metadata. Encoders accept pixels as-is and embed the provided
metadata. The caller handles CMS transforms.

---

## Shared native mapping and ownership

`zencodec::frame` owns the existing `Frame`, `Picture`, `MappedPlanes`,
`PlaneView`, `PlaneSamples`, `MapError`, `PlaneError` and `Presentation`
contracts. `zencodec_media::frame` re-exports those exact types. Frames retain
immutable interpretation and an owned picture after decoder reset/drop; another
presentation can share that picture with different timing. Mapped rows borrow
the native guard and preserve word stride, significant code bits and bit shift.
A native ten-bit word remains a component code, not a normalized RGB U16 sample.
`PlaneView::as_strided_samples` borrows the complete declared span for existing
plane kernels: it includes inter-row padding and ends after the last active
sample. Empty geometry exposes no storage. The accessor neither copies nor
applies the sample encoding; consumers must still honor code bits and shift.

```rust
trait Picture: Send + Sync {
    fn map(&self) -> Result<Box<dyn MappedPlanes + '_>, MapError>;
}
trait MappedPlanes {
    fn plane_count(&self) -> usize;
    fn plane(&self, index: usize) -> Option<PlaneView<'_>>;
}
```

The core has no media or I/O-driver dependency. Accounted software `OwnedPlanes`
remain in media, while packed image interchange remains `zenpixels::PixelSlice`.
The shared collection and checked consumer are implemented on the API review
PR; concrete native factories remain in the validation harness. Mapping does not certify
display conversion, whole-container metadata treatment or a native workspace
bound. See [the media contract](media-contract.md#frames).

### Mapped image output (`zencodec::image`)

`ImageOutput` owns packed pixels plus auxiliary values, or an existing native
component `Frame`. `map` borrows planes through a guard. `with_pixels` is the
packed-color convenience and refuses component codes. Each plane distinguishes
`Samples::{Pixels, Values, Codes}` and `Meaning::{Color, Auxiliary, Component}`;
auxiliary scalar values are not relabeled grayscale color.

`ImageInterpretation` is frozen with the owner. It retains stored canvas/region,
residual orientation, optional native signal/timing and opaque typed source
details. Its optional `composition` distinguishes a qualified composited view
from a raw layer; packed storage or equal dimensions do not establish that fact.
The checked collection binds its view before publication, preserving existing
clones and sample storage. A contradictory native claim, or a view that changes
during decoding, returns `OutputError::Composition { expected, actual }` before
the policy callback. Standalone constructors leave the view unknown unless the
adapter supplies it. This does not certify blending arithmetic, orientation,
color conversion or display readiness. `ImageFacts::composition` exposes the
same bounded fact to the existing resolver. `Auxiliary` separately retains source precision and source dimensional
shift. Spot color records whether its contribution is already `Applied` or
still `Separate`; retaining its samples does not authorize applying it again.
`AuxiliaryKind::Black { inverted }` declares ink polarity rather than assuming
an attached ICC profile establishes it. Native JXL uses inverted black and the
`zenpixels::PixelFormat::CmykF32Inverted` packed format; conventional device values
use `CmykF32`. These are different interpretations of the same physical layout,
and an unqualified conversion must refuse.

`ImageInterpretation::placement: Option<Placement>` describes the selected
image's signed origin on a separate presentation canvas, before orientation.
`Placement::visible(geometry)` returns matching sample-buffer and canvas
rectangles, or `None` for valid fully off-canvas samples. It never clips the
owner. `ImageOutput::packed` rejects an empty presentation canvas or signed
overflow as `OutputError::Placement`, retaining `PlacementError` as its cause.
Placement does not establish blend instructions or authorize discarding samples.
`ImageFacts` retains it for bounded diagnostics; `Question::Image` borrows those
facts for the callback instead of embedding another full copy in the question.

```rust
impl ImageOutput {
    fn packed(pixels: zenpixels::PixelBuffer, extras: Vec<(Auxiliary, Values)>,
        interpretation: ImageInterpretation) -> Result<Self, At<OutputError>>;
    fn from_packed_owner<P: PackedImageOwner + 'static>(owner: P,
        auxiliaries: Vec<Auxiliary>, interpretation: ImageInterpretation)
        -> Result<Self, At<OutputError>>;
    fn replace_packed_color<P: PackedImageOwner + 'static>(self, owner: P)
        -> Result<Self, At<OutputError>>;
    fn from_frame(frame: frame::Frame) -> Self;
    fn from_frame_pixels<P: PackedImageOwner + 'static>(frame: frame::Frame,
        owner: P, orientation: zenpixels::Orientation) -> Result<Self, At<OutputError>>;
    fn source_frame(&self) -> Option<&Arc<frame::FrameInterpretation>>;
    fn interpretation(&self) -> &ImageInterpretation;
    fn map(&self) -> Result<Box<dyn MappedImage + '_>, At<OutputError>>;
    fn with_pixels<T>(&self, consume: impl FnOnce(zenpixels::PixelSlice<'_>) -> T)
        -> Result<T, At<OutputError>>;
}
trait MappedImage {
    fn plane_count(&self) -> usize;
    fn plane(&self, index: usize) -> Result<Option<Plane<'_>>, At<OutputError>>;
}
trait PackedImageOwner: Send + Sync {
    fn pixels(&self) -> Result<zenpixels::PixelSlice<'_>, At<OutputError>>;
    fn auxiliary(&self, index: usize)
        -> Result<Option<ValueSlice<'_>>, At<OutputError>>;
    fn retain_auxiliaries(&self)
        -> Result<Option<Arc<dyn AuxiliaryImageOwner>>, At<OutputError>>;
}
trait AuxiliaryImageOwner: Send + Sync {
    fn auxiliary(&self, index: usize)
        -> Result<Option<ValueSlice<'_>>, At<OutputError>>;
}
```

`ImageOutput::from_frame_pixels` adopts an explicitly converted packed owner for
the native frame's exact visible crop. It retains timing and presentation while
moving the original component interpretation to `source_frame()`. Current
`interpretation().signal` is absent: the packed descriptor/context describe
current samples. `ImageFacts::source_frame` exposes bounded source facts to the
existing publication resolver, distinct from native-current `ImageFacts::frame`.
The consumed native picture is released; retained clones remain independent.
Color replacement preserves that provenance and temporal identity. This adapter
entry point does not execute or certify conversion, infer orientation, rescale
to a signaled render size, or authorize metadata transfer. The caller supplies
residual orientation explicitly. Packed constructors still reject current component claims and presentation
facts without explicit source-frame provenance. That provenance is part of
`ImageInterpretation`, so a subsequent transform can clone it without losing
timing or source evidence.

`PackedImageOwner` retains an immutable native or admitted allocation through the
last `ImageOutput` clone. `ValueSlice` borrows strided auxiliary storage, so an
adapter can publish already-admitted buffers without converting them to `Vec`.
`Samples::Values` holds that view by value. Construction checks every declared
plane; mapping refuses changes to dimensions, stride, descriptor or color context.
The implementor must also preserve sample contents. This is an ownership
mechanism; it adds no policy callback and does not admit existing allocations
retroactively. Native workspace and small owner/metadata allocations are outside
a managed payload allowance. [Native qualification](../perf/spatial-validation/README.md#packed-output-admission)
exercises release on refusal/cancellation and retention through clones.

`Values` and `ValueSlice` carry `AuxiliaryGrid`, defaulting to `Unknown`.
An adapter can declare `ColorSamples` only when each emitted auxiliary sample
has the same center as the color sample at the corresponding row and column
throughout the selected stored region. Adoption checks matching dimensions;
mapping freezes the claim, including through independent auxiliary ownership
and color replacement. Equal sizes, source precision and `source_dim_shift`
never establish registration. `with_grid` changes no samples and performs no
resampling. This initial positive case qualifies same-grid geometry operations;
arbitrary offsets, sampling steps and phases still need a separate representation
before general auxiliary resampling can be offered.

`replace_packed_color` composes converted color with the original auxiliary
mapping guards. The replacement must cover the same stored coordinates. It
retains geometry, placement, orientation, timing, auxiliary roles and original
source facts; its descriptor/context describe the new color. It neither
transforms nor renders auxiliaries, and it refuses component-code output.
`PackedImageOwner::retain_auxiliaries` optionally supplies an independent owner
of the same scalar planes. Replacement calls it automatically, verifies the
frozen layouts and retains its traced failures. The source color can then
release when no original image clone retains it. `None` keeps the complete
source owner, including its original color charge; it never triggers a copy.
Repeated replacement releases intermediate color owners and keeps the already
retained auxiliary owner without invoking the hook again. With no
auxiliaries, the consumed source storage releases unless another clone holds it.
Publication still uses the existing resolver. The optional owner mechanism does
not require caller capability coordination or a second policy callback.

Packed construction checks region bounds, pixel dimensions and compatible
interpretation. Frame mapping checks the stored/crop extent, role count and
mapped per-plane dimensions/encoding. It retains native mapping errors in the
source chain. It does not scan sample codes, validate ICC profiles or establish
metadata privacy. Names, opaque profiles and typed source details remain
unfiltered data; policy admission is a separate boundary.
Mapping-error `Display` and `Debug` stay bounded; explicit cause inspection
retains the original backend error and trace, which may contain private data.

## Image collections (`zencodec::collection`)

An opened collection fixes its source operation and composited/uncomposited view.
`ImageId` is opaque, clonable, comparable and hashable within that operation;
it is not a portable index or proof of source consistency. `Origin` lets an
adapter issue IDs and reject foreign ones. Cursor ownership is also checked by
the adapter. Containment is separate from reduced levels and video timing.

```rust
trait Collection {
    type Cursor;
    fn view(&self) -> Composition;
    fn images(&mut self) -> Result<Self::Cursor, At<CollectionError>>;
    fn children(&mut self, image: &ImageId) -> Result<Self::Cursor, At<CollectionError>>;
    fn next(&mut self, cursor: &mut Self::Cursor) -> Result<Option<ImageId>, At<CollectionError>>;
    fn describe(&mut self, image: &ImageId) -> Result<ImageDescription, At<CollectionError>>;
    fn decode(&mut self, image: &ImageId, request: &ImageRequest)
        -> Result<ImageOutput, At<CollectionError>>;
}
```

`ImageRequest::new()` requests the full selected image. `with_region(Rect)`
requires that exact nonempty stored region. Unsupported views, containment and
regions return typed errors; they cannot silently become flattened pixels,
empty discovery or a clipped region. Backend failures retain their original
concrete error and location trace; default formatting omits backend payloads.

`Discovery<C>::new(collection, attempts)` applies one immutable call allowance
across all cursors. It charges before delegating `images`, `children`, `next` and `describe`,
including cached results, exhaustion and errors. It does not count native parser
work, source bytes, allocation or CPU time. Issued IDs remain decodable after
discovery exhaustion.

`ImageDescription` contains local stored dimensions, optional established
orientation and separate placement. Description does not reconstruct pixels or
advance a cursor. `CheckedCollection::describe` validates these facts and checks
the operation's Pulse before returning them; errors identify the `Describe`
stage. This query establishes no rendered format, supported compression,
complete metadata or resource admission. A later decode can still fail.

`operation::CheckedCollection<C, R>::open(pulse, resolver, factory)` passes one
`SharedPulse` with latched cancellation to the factory and checks cancellation
before and after requests. Its
`first_image` and `decode` return `Outcome<ImageOutput>` only after the existing
policy accepts that exact held owner. Pass `PolicySet` or `BoundPolicy` as the
operation policy. Deprecated `with_policy_set` requirements still apply;
`report` retains bounded diagnostics. Region/policy refusals are
request-local; cancellation or unwind drops the native collection permanently.
Native terminal errors remain the adapter's duty. `CollectionOperationError`
identifies the failure stage and retains the original cause through `Error::source`.
Its `stop_reason()` accessor reports the first stop observed by the operation's
shared token across opening, native and policy failures, and subsequent finished
calls. Coincident cancellation does not replace the original failure. `None`
means no stop was observed through that token, rather than classifying all native
or application errors as non-cancellation.

`CheckedCollection::decode_with(image, native_request, transform)` adds an
adapter processing boundary before that single publication decision. It passes
the bound owned image and the same `&dyn Pulse` to a fallible `FnOnce`.
The completed `ImageOutput` reaches the resolver; a failed transform does not.
`CollectionStage::Transform` preserves the original traced processing cause.
The transform must honor its configured requirements and update interpretation
with its samples; the hook does not independently prove its math or workspace.
Native `decode` delegates through the same path with an identity transform.
No qualified collection/transform workspace bound is available: an explicit
workspace limit refuses before the collection factory and before subsequent
native calls. Both bound and legacy requirements apply, and refusal retains
`UnmetRequirements` with `Workspace(Unknown)`. A request-local refusal preserves
the existing collection owner.

`CodecSet::with_collection(factory)` registers immutable `CollectionFactory`
configurations. `open_collection(format, source, request, resolver, pulse)` returns
`CheckedCollection<Discovery<ErasedCollection<'a>>, R>`, borrowing the positional
source without borrowing the registry. The first matching registration wins,
including failures; no implicit format detection, source staging or fallback.
The factory receives the matched format, required view, inherited resource limits
and decode policy, and the retained `SharedPulse` with latched cancellation. The explicitly encode-only
output-byte limit does not constrain collection decoding. It must honor configured
requirements or refuse before affected work. This is stronger than the legacy
best-effort policy setter on still-image jobs. The inherited cancellation
condition and per-call `SharedPulse` remain active after the registry drops.
Opening preserves the supplied Pulse; it does not itself add native progress
reporting.

```rust
trait CollectionFactory: Send + Sync {
    fn formats(&self) -> &[ImageFormat];
    fn open_with<'a>(&self, source: &'a mut dyn DynReadAt,
        context: CollectionOpenContext) -> Result<ErasedCollection<'a>, Cause>;
}
impl OpenRequest {
    fn new(view: Composition, discovery_attempts: u64) -> Self;
    fn with_output_storage(self, storage: storage::StorageBudget) -> Self;
}
```

`CollectionOpenContext` groups matched format, view, inherited limits/decode
policy, the retained `SharedPulse` with latched cancellation and optional required output storage. The
operation pool is shared by request clones; admission must precede native output
allocation, and charges remain with the last retained image owner. This does not
cover native workspace or metadata. Adapters that cannot enforce the pool return
`CollectionOpenError::UnsupportedOutputStorage` before affected work. There is
one required native opening method; no alternate compatibility entry remains.
The packaged JXL factory consumes the request pool without a codec-specific
storage setter. The private TIFF factory refuses this requirement before source
queries. A pool never satisfies an unqualified native workspace limit.

`ErasedCollection::new` erases the native collection once and its owned cursor
state once per cursor. Foreign cursors fail before native work. The small box
allocations are infallible; the erased job is not promised `Send`. Native image
IDs and output ownership remain unchanged. Typed collections remain available.
`CollectionOpenError` distinguishes absent registration, resident-input needs,
unimplemented limit enforcement and unimplemented decode-policy enforcement.
Its `UnsupportedLimits { unsupported: ResourceLimits }` and
`UnsupportedDecodePolicy { unsupported: DecodePolicy }` variants retain the
unhandled requirements. Qualified or irrelevant fields have no-op/default
values. This bounded data is diagnostic; it never authorizes a weaker retry.

This is publication after reconstruction, not native allocation admission or a
whole-container privacy guarantee. Concrete adapter packaging, qualified native
configuration mappings and broader source/workspace bounds remain open. See
[the collection contract](collection-contract.md) for executing native coverage,
caller examples and the full failure boundaries.

## Stored operation requests

`DecodeRowSink::max_buffer_rows() -> core::num::NonZeroU32` reports the sink's
admitted strip capacity after `begin`. Its default is unconstrained. Shared
`helpers::copy_rows_to_sink` and coroutine delivery divide larger native batches,
preserving absolute row coordinates and provisional-pass metadata. This bounds
sink buffers only: native reconstruction and a coroutine's held batch may be
larger. Existing codecs that request larger buffers can still receive a typed
sink failure; bounded sinks must not grow silently. One native progress event
can represent several consecutive sink buffer requests.

`zencodec::operation` records intent and checks it during execution. These
additive entry points use the same registered templates and native jobs as
`CodecSet`. Builders do not query capabilities. A required policy uses the
checked setter, including an inherited set policy; explicit request policies
replace that default. Required output rejects a native fallback while an
explicit preference permits it. Fidelity confirms configuration selection,
with the existing content-dependent codec restrictions still enforced at encode.

```rust
struct DecodeRequest { /* private */ }
impl DecodeRequest {
    const fn new() -> Self;
    const fn with_format(self, format: ImageFormat) -> Self;
    const fn with_policy(self, policy: DecodePolicy) -> Self;
    fn with_output(self, descriptor: PixelDescriptor) -> Self;
    fn preferring_output(self, descriptors: impl Into<Vec<PixelDescriptor>>) -> Self;
    fn start_controlled(
        &self, codecs: &CodecSet, storage: storage::StorageBudget,
    ) -> Result<Box<decode::DynControlledDecoder>, At<operation::OperationError>>;
    fn decode(&self, codecs: &CodecSet, input: &[u8]) -> Result<Outcome<DecodeOutput>, At<OperationError>>;
}
struct EncodeRequest { /* private */ }
impl EncodeRequest {
    const fn new(format: ImageFormat) -> Self;
    fn with_fidelity(self, fidelity: Fidelity) -> Self;
    fn preferring_fidelity(self, targets: impl Into<Vec<Fidelity>>) -> Self;
    const fn with_policy(self, policy: EncodePolicy) -> Self;
    const fn with_policy_defaults(self, defaults: EncodePolicy) -> Self;
    const fn with_policy_set(self, preset: PolicySet) -> Self;
    const fn with_resolution(self, resolution: Resolution) -> Self;
    fn prepare(&self, codecs: &CodecSet) -> Result<Vec<PreparedEncode>, At<OperationError>>;
    fn prepare_send(&self, codecs: &CodecSet) -> Result<Vec<PreparedSendEncode>, At<OperationError>>;
    fn encode(&self, codecs: &CodecSet, pixels: PixelSlice<'_>) -> Result<Outcome<EncodeOutput>, At<OperationError>>;
    fn encode_with<R: OperationPolicy + ?Sized>(&self, codecs: &CodecSet,
        pixels: PixelSlice<'_>, resolver: &mut R) -> Result<Outcome<EncodeOutput>, At<OperationError>>;
}
struct PreparedEncode { /* private; Debug */ }
impl PreparedEncode {
    const fn offer(&self) -> EncodeOffer;
    fn into_job(self) -> Box<dyn encode::DynEncodeJob>;
}
struct PreparedSendEncode { /* private; Debug + Send */ }
impl PreparedSendEncode {
    const fn offer(&self) -> EncodeOffer;
    fn into_job(self) -> Box<dyn encode::DynEncodeJob + Send>;
}
struct Outcome<T> { /* private */ }
impl<T> Outcome<T> { // Also Deref<Target = T>.
    const fn from_parts(output: T, report: OperationReport) -> Self;
    const fn output(&self) -> &T;
    fn into_output(self) -> T;
    fn into_parts(self) -> (T, OperationReport);
    const fn report(&self) -> &OperationReport;
}
enum OperationError {
    Codec(CodecSetError),
    AlternativeCount { count: usize, limit: usize },
    InvalidFidelity { preference_index: usize, cause: FidelityTargetError },
    NoEncodeRoute(Vec<RouteRejection>),
    Allocation,
    Stopped(enough::StopReason),
    Policy(PolicyFailure),
    Execution { cause: Box<CodecSetError>, report: Box<OperationReport> },
    OutputDescriptor { requested: PixelDescriptor, actual: PixelDescriptor },
}
```

Transferable preparation requires registration with
`CodecSet::with_send_encoder(config)`, where `Config::Job: Send`. Ordinary
`with_encoder` remains available for thread-affine jobs. Both registrations use
the same first-registration-wins lookup and retain their behavior on set clones.
The send path stamps the same required policy, limits and cancellation controls
before exposing an offer; it does not rebuild the chosen job on another thread.

### One application resolver

The ordinary `encode` call uses its configured `PolicySet`. `encode_with` accepts
`PolicySet`, `BoundPolicy`, a compatible operation-policy trait object, or a closure.
Custom callback objects implement `PolicyResolver` and are passed inside
`BoundPolicy::new(requirements, callback)`; a callback object alone is not an
`OperationPolicy`.
The callback returns `Result<Decision, At<E>>`; the native `At<E>` survives in
the error chain, including its trace. Annotate a closure argument as
`question: Question<'_>` so Rust infers its borrowed-question lifetime correctly.

```rust
trait PolicyResolver {
    type Error: core::error::Error + Send + Sync + 'static;
    fn resolve(&mut self, question: Question<'_>) -> Result<Decision, At<Self::Error>>;
}
enum Question<'a> { // non_exhaustive
    EncodeRoute { declaration: &'a EncodeDeclaration<'a>, plan: &'a EncodePlan,
        facts: EncodeRouteFacts, choices: Choices<'a, EncodeRouteOffer> },
    OutputSchedule { facts: OutputScheduleFacts,
        choices: Choices<'a, OutputScheduleOffer> },
    ImageConversion { image: &'a image::ImageOutput, facts: &'a ImageFacts,
        choices: Choices<'a, ImageConversionOffer> },
    FrameRoute { interpretation: &'a frame::FrameInterpretation,
        facts: &'a FrameRouteFacts, choices: Choices<'a, FrameRouteOffer> },
    Image { image: &'a image::ImageOutput, facts: ImageFacts,
        choices: Choices<'a, PublicationOffer> },
    Transcode { facts: InterpretationFacts, choices: Choices<'a, TranscodeOffer> },
    Boundary { facts: InterpretationFacts, choices: Choices<'a, BoundaryOffer> },
    Encode { facts: EncodeFacts, choices: Choices<'a, EncodeOffer> },
    Frame { interpretation: &'a frame::FrameInterpretation,
        timing: frame::FrameTiming, choices: Choices<'a, PublicationOffer> },
}
struct BoundaryOffer { action: BoundaryAction }
struct EncodeFacts { width: u32, height: u32, descriptor: PixelDescriptor }
struct EncodeOffer {
    format: ImageFormat, fidelity: Option<Fidelity>, preference_index: usize,
    resolution: Option<Resolution>,
}
struct ChoiceId { /* opaque, scoped to one question */ }
struct Choice<'a, T> { /* private */ }
impl<'a, T> Choice<'a, T> {
    const fn value(self) -> &'a T;
    const fn id(self) -> ChoiceId;
}
struct Choices<'a, T> { /* private */ }
impl<'a, T> Choices<'a, T> {
    const fn len(self) -> usize;
    const fn is_empty(self) -> bool;
    fn first(self) -> Option<Choice<'a, T>>;
    fn iter(self) -> impl ExactSizeIterator<Item = Choice<'a, T>>;
}
enum Decision { Choose(ChoiceId), UsePreset, Decline(At<Decline>) }
impl Decision {
    fn choose<T>(choice: Choice<'_, T>) -> Self;
    fn decline(reason: Decline) -> Self;
}
enum Decline { ApplicationRule, NoPreferredChoice, UnsupportedQuestion }
struct PolicySet { /* private */ }
impl PolicySet {
    const fn new() -> Self;
    const fn prefer_lossless(self, enabled: bool) -> Self;
    const fn require_metadata(self, treatment: MetadataTransfer) -> Self;
    const fn allow_opaque_profiles(self, allowed: bool) -> Self;
    const fn with_workspace_limit(self, bytes: u64) -> Self;
    const fn allow_assumed_chroma_siting(self, allowed: bool) -> Self;
    const fn require_extended_range(self, required: bool) -> Self;
    const fn require_output_schedule(self, schedule: OutputSchedule) -> Self;
    const fn with_output_wait_limit(self, bytes: u64) -> Self;
    const fn with_max_source_passes(self, passes: u32) -> Self;
    const fn require_exact_replay(self, required: bool) -> Self;
    const fn allow_raw_memory_staging(self, allowed: bool) -> Self;
}
```

Offers are prepared encoder jobs whose fidelity configuration and required
policy were checked. The selected job survives the callback unchanged; the
other jobs are dropped before encoding. Malformed targets reject the entire
request; unsupported targets may be omitted from a preference list. The planner
accepts 1–64 targets, reserves its list storage fallibly, and never retries an
execution failure with another target. Preparation can allocate inside codecs;
the offer limit is not a total-memory guarantee.

### Mandatory admission before callback selection

`PolicySet` requirements now constrain every callback, including direct
`Decision::Choose`. Metadata treatment, opaque-profile exclusion, workspace,
extended range, output schedule, output-wait limits, external source passes, exact
replay and raw RAM staging restrictions are checked before choices
reach application code. `UsePreset` selects among the admitted choices; lossless
preference and the decision to accept a disclosed chroma-siting assumption remain
situational. Required request configuration still applies independently.

Admission retains the original offer indices and prepared jobs. It uses a bounded
bit mask, without copying offers or allocating a filtered list. When requirements
exclude every executable alternative, the operation refuses before invoking the
callback. An originally empty executable set retains its existing callback
behavior. Stale tokens still cannot select a route from another question.

`DecisionRecord::offered` remains the number of supplied alternatives before
admission; `admitted` counts the alternatives the callback can select.
`unmet_requirements: UnmetRequirements` contains the union of failures across
excluded alternatives. Its iterator yields `(PolicyRequirement,
RequirementEvidence)` pairs: `Unknown` or `Contradicted`. Different alternatives
can fail different requirements. A successful selection can retain failures from
other, excluded alternatives. Default failure text names these requirement codes
when admission leaves no route; no metadata payload is included.

Scope is deliberately conservative: a domain lacking evidence cannot silently
mark a requirement not applicable. Only interpretation discovery defers checking
until final adoption, before affected output. This does not bound discovery
allocations. Reusable tenant policy across coupled operation stages still needs
explicit requirement scope and discharge; admission alone does not implement it.
Do not remove `TranscodeRequest::with_metadata` in favor of preset requirements
until that single-source contract is implemented and qualified.

Tokens from another question, including a nested operation, are refused.
`UsePreset` delegates once. A callback panic unwinds this one-shot operation and
drops prepared jobs; a caught panic does not leave an operation to resume.
Cancellation is checked before preparation and on both sides of the callback.
This does not make uncooperative native encoding preemptible.

`DecisionRecord::context` retains typed `DecisionContext` facts even on refusal;
`InterpretationFacts::unavailable` carries an optional `InterpretationUnavailable`
code without metadata payloads. `with_unavailable(reason)` marks the facts unavailable.

`OperationReport` keeps the latest 16 `DecisionRecord`s and counts omitted older
records. A record carries question identity, offer count, selected typed contract
(`Selection::OutputSchedule`, `Selection::ImageConversion`, `Selection::FrameRoute`,
`Selection::Image`, `Selection::Frame`, `Selection::Transcode`, `Selection::Encode`
or `Selection::Boundary`),
preset delegation and `DecisionCode`. Adapters append records with
`OperationReport::record(record)`.
The fixed history overwrites its oldest slot without moving retained records;
iteration and debug formatting remain chronological. The omitted count saturates
at `u64::MAX`. [Frame-policy measurements](../benchmarks/frame_policy_2026-10-25.md)
cover the actual resolver and report separately.
Success returns this report with the output; native execution failure retains it
in `OperationError::Execution`. `PolicyFailure::record()` identifies refusal,
stale choice, callback failure or cancellation. Default policy-failure formatting
excludes application error payloads; explicit source/trace inspection can reveal
them. Native codec errors keep their codec's own formatting.

The full implementation target and current gaps are recorded in
[ideal-api.md](ideal-api.md). The resolver serves prepared encoder configuration, mandatory interpretation
boundaries, native frame publication and complete pixel-transcode routes. The executing transcode adapter
is in zencodecs. The pinned JPEG/PNG guide routes are qualified; other native
routes and configurations need separate qualification.

### Prepared image conversion

```rust
struct ImageConversionOffer { // non_exhaustive
    output: PixelDescriptor, geometry: image::ImageGeometry,
    orientation: Orientation, assumed_chroma_siting: Option<frame::ChromaSiting>,
    preserves_extended_range: bool, admitted_bytes: usize,
    workspace_upper_bound: Option<u64>,
}
fn resolve_image_conversion<R: OperationPolicy + ?Sized>(
    image: &image::ImageOutput, offers: &[ImageConversionOffer], resolver: &mut R,
    preset: PolicySet, pulse: &dyn Pulse,
) -> Result<(usize, DecisionRecord), At<PolicyFailure>>;
```

Adapters hold the exact source mapping, conversion mechanism and resource
reservations while the existing callback selects a route. Selection precedes
pixel writes and destination initialization. The selected route executes once;
there is no hidden fallback or plan reconstruction. Execution errors retain the
selected `Selection::ImageConversion` record and the original cause.
`DecisionContext::ImageConversion` preserves bounded source facts on refusal.
The shared resolver validates decision tokens and cancellation, not conversion
math, native mapping or adapter claims.

The default preset refuses an assumed chroma location. Explicit permission only
allows a disclosed concrete location when source siting is undeclared; it never
permits replacing a signaled location. `require_extended_range(true)` accepts
only a route qualifying that guarantee. Other output questions lack this
proof and refuse; discovery boundaries may continue. This is neither a lossless
precision guarantee nor an HDR rendering rule. Required metadata/opaque-profile
policies refuse conversion-only offers, which cannot prove those obligations.

`admitted_bytes` includes retained destination/scratch reservations, not just
already allocated pixels. `StorageBudget::prepare_filled(len, value)` returns
`PreparedStorage<T>` without creating the array. Its consuming `allocate()`
transfers the same charge into `OwnedStorage<T>`; dropping it releases the charge.
Allocation may still fail. `filled`, `bytes`, `words` and `floats` retain their
immediate-allocation convenience. Neither admission nor a qualified additional
workspace bound establishes total operation memory.

The private native AV1 witness executes this boundary for strided RGB16 output,
with one coded RGB row of admitted scratch. A second explicit target executes
unclipped RGB F32 with no managed row scratch and unchanged sRGB transfer.
The same offer and callback suffice. Native crops are bit-identical to full-frame
rows; independent zimg agreement has a fixed 1e-6 absolute gate for the tested
BT.709/sRGB cases. The same native frames/crops now execute existing OutputPlan
linear-light processing and packed-owner replacement with exact row-batch agreement.
Its continuous sRGB definition differs measurably from IEC constants; explicit
unclipped options are required. The float mechanism now uses the public
`zenpixels_convert::ycbcr::YcbcrToRgbF32` and borrowed `sample::CodePlane` on
the pinned pixel review. Its errors retain actual refusal facts. Other
matrices/transfers, non-centered siting, integer output and HDR rendering remain
unqualified. See the
[native qualification](../perf/av1-policy-validation/README.md#prepared-conversion-before-allocation).

### Prepared frame encoding transitions

`Question::FrameRoute` asks the same fallible resolver before accepting a held
frame into a prepared encoder. The adapter retains the actual candidate encoder,
held input and old encoder during the decision. It drains old output before
installing a replacement. Input epochs remain provenance; output segments have
independent configuration epochs. Changing only the input epoch does not imply
an output transition.

```rust
enum FrameRoute { Continue, BeginSegment } // non_exhaustive
struct FrameRouteState {
    previous: Option<FrameFacts>,
    completed_segments: u64,
    output_committed: bool,
}
struct FrameRouteFacts { input: FrameFacts, output: FrameRouteState }
struct FrameRouteOffer {
    route: FrameRoute,
    fidelity: Option<Fidelity>,
    conversion: Option<FrameConversionOffer>,
    admitted_encoded_bytes: usize,
    workspace_upper_bound: Option<u64>,
}
struct FrameConversionOffer {
    encoder_input: FrameFacts,
    preserves_samples: bool,
    preserves_extended_range: bool,
    assumed_chroma_siting: Option<frame::ChromaSiting>,
    admitted_bytes: usize,
}
fn resolve_frame_route<R: OperationPolicy + ?Sized>(
    interpretation: &frame::FrameInterpretation,
    timing: frame::FrameTiming,
    output: FrameRouteState,
    offers: &[FrameRouteOffer],
    resolver: &mut R,
    preset: PolicySet,
    pulse: &dyn Pulse,
) -> Result<(usize, DecisionRecord), At<PolicyFailure>>;
```

The destination registration is already fixed by the request. `Continue` keeps
its current interpretation; `BeginSegment` starts an independently configured
segment only when the request and destination permit it. Neither authorizes
in-band container reconfiguration. The callback cannot manufacture an unsupported
route. `Selection::FrameRoute` and `DecisionContext::FrameRoute` retain the selected
contract and before-decision facts in the bounded report.

`output_committed` includes externally visible headers or representation choices,
not just encoded body bytes. It is adapter evidence, not a reservation or proof
of destination support. A publication attempt that fails after partial effects
must terminate the operation; private storage cleanup cannot undo those effects.
Unknown workspace refuses a required workspace limit, and lossless preference
never overrides that limit. This offer has no privacy evidence: presets requiring
metadata treatment or excluding opaque profiles refuse it.

The [native AV1 witness](../perf/av1-policy-validation/README.md#prepared-output-transitions)
qualifies fixed output refusal and finite segmented encoding. It releases native
encoders before returning staged compressed packets to a possibly slow writer.
The admitted bytes cover compressed payloads, not codec workspace, metadata,
packet bookkeeping or transient native output. This is not a general bounded
spool or a live sequential-output qualification. Prepared planar conversion and
continuous-container configuration changes remain separate contracts.

`FrameRouteOffer::conversion` couples a retained converter to that encoder.
`None` means unqualified, not identity. Original input facts stay in the
question; `encoder_input` names the actual post-conversion interpretation and
timing. The adapter retains the mapping, plan, reservations and encoder through
one callback, executes only the selected plan, and checks its output before
encoder acceptance. Conversion failure is terminal and retains the selected
record. `fidelity` still describes the encoder alone: a sample-preserving
transcode requires both a preserving conversion and lossless encoding.
`prefer_lossless(true)` ranks that complete evidence first, then lossless encoder
mode alone, then request order. It remains a preference and never bypasses a
required workspace or range constraint.

The range preset accepts a frame route only with both qualified range-preserving
conversion and confirmed lossless encoding. Siting assumptions also require
permission. Unknown workspace and metadata/privacy obligations remain separate
refusals. These fields are adapter evidence, not runtime verification of math.
The native coupled witness normalizes changing BT.709/sRGB YCbCr input to fixed
RGB8 identity-matrix AV1, with explicit saturation/quantization disclosed as
non-preserving. This does not qualify HDR, resizing, retiming or muxer changes.

### Coupled transcode offers

```rust
enum MetadataTransfer { PreserveReported, DiscardDescriptive } // non_exhaustive
struct TranscodeOffer { // non_exhaustive; fields public
    encoder: EncodeOffer,
    output: PixelDescriptor,
    metadata: MetadataTransfer,
    retains_opaque_profile: Option<bool>,
    admitted_bytes: usize,
    workspace_upper_bound: Option<u64>,
    output_schedule: Option<OutputScheduleOffer>,
}
fn resolve_transcode<R: OperationPolicy + ?Sized>(
    boundary: DecodeBoundary, offers: &[TranscodeOffer], resolver: &mut R,
    preset: PolicySet, pulse: &dyn Pulse,
) -> Result<(usize, DecisionRecord), At<PolicyFailure>>;
```

The adapter retains actual conversion/encoder objects across the callback and
executes the selected index. The source must be suspended with final usable
interpretation; provisional or unavailable facts offer no route. This function
checks choice authority, not the truth of an adapter's resource claims.
`retains_opaque_profile` defaults to `None` (unknown). A preset forbidding opaque
profiles accepts only `Some(false)` supplied by the executing adapter; absence
of inspection is not evidence of absence. This does not certify unknown packets,
nested images, XML, or personal information visible in the pixels.
`admitted_bytes` is the route's managed row/conversion storage; alternatives may
share it. `workspace_upper_bound: None` means unknown. The built-in workspace
limit rejects unknown bounds and excludes source, managed and encoded storage.
It is not a total RAM cap.

`PolicySet` can require descriptive metadata removal and forbid retaining opaque
source profiles. Encode-only questions lack that evidence and refuse these
requirements. Neither setting certifies pixels or unknown/nested carriers as
PII-free. `Outcome::from_parts` assembles an adapter's result and report; the
adapter remains responsible for validating postconditions.

## Controlled decode adapters

The [controlled protocol](controlled-decoding.md) adds
`ControlledIncrementalDecode::step_controlled` and `resume` alongside the
legacy incremental methods. `ControlledProgress::Suspended` holds an opaque
`BoundaryId` and `InterpretationFacts`; it is separate from `Event::Metadata`
and from byte `InputRequest`s. `erase_controlled` preserves native errors and enforces
suspension, cancellation and terminal failures through `Fused`.

```rust
// zencodec::operation: adapter entry point, not a caller coordination step
fn resolve_boundary<R: OperationPolicy + ?Sized>(
    boundary: DecodeBoundary,
    resolver: &mut R,
    preset: PolicySet,
    pulse: &dyn Pulse,
) -> Result<(BoundaryAction, DecisionRecord), At<PolicyFailure>>;

// zencodec_io: runs with the existing slice/read-at/stream/async drivers
struct PolicyDecoder<D, R> { /* private */ }
impl<D, R> PolicyDecoder<D, R> {
    fn new(decoder: D, resolver: R) -> Self;
    fn with_policy_set(self, preset: PolicySet) -> Self;
    fn report(&self) -> &OperationReport;
}

// zencodec_io: adapters that must prepare other stages while suspended
enum ControlledEvent {
    Decode(decode::Event),
    Suspended(decode::DecodeBoundary),
}
struct ControlledStepper<'a, D, S> { /* private */ }
impl<'a, D: decode::ControlledIncrementalDecode, S: decode::DecodeRowSink>
    ControlledStepper<'a, D, S>
{
    fn new(decoder: D, sink: &'a mut S) -> Self;
    fn info(&self) -> Option<&ImageInfo>;
    fn output_descriptor(&self) -> Option<zenpixels::PixelDescriptor>;
    fn sink(&self) -> &S;
    fn sink_mut(&mut self) -> &mut S;
    fn resume(&mut self, boundary: decode::BoundaryId,
        action: decode::BoundaryAction, pulse: &dyn Pulse)
        -> Result<(), decode::StepError<D::Error>>;
}
// ByteStepper<Output = ControlledEvent, Error = decode::StepError<D::Error>>
```

`ControlledStepper` runs through `SliceCursor`, `ReadAtCursor`, `ReadCursor`
and feature `tokio`'s `AsyncCursor`. It exposes mandatory boundaries to an
adapter that needs to prepare a destination before approving pixel work.
`sink_mut` permits installing that destination while keeping the native
session held. Repeated cursor calls return the same boundary without reads
or native progress. `resume` performs the protocol transition; the following
cursor call performs native work. Native/sink error, cancellation or unwind
drops the native session even if the caller retains the stepper. Source
errors belong to the cursor; the adapter must drop the stepper when abandoning
an operation. This primitive does not itself prepare a transcode route.

Codec authors can implement the protocol directly or use
`zencodec_coroutine::CoroutineDecoder::controlled(input_cap, access, init)`.
`controlled_with_storage(input_cap, access, StorageBudget, init)` additionally
admits input payload, its bounded range index and row staging. It returns
`Result<Self, CoroutineError>` and admits initial storage before calling `init`.
The coroutine awaits `ctx.interpretation(facts)` before affected work. It
refuses output before final adoption and refuses changed final interpretation.
See [the authoring contract](controlled-decoding.md#coroutine-authoring-and-transport-execution)
for enforcement and current storage scope.

`PolicyDecoder` owns the controlled decoder and callback. Failure or unwind drops
the native session while retaining its decision report; native metadata accessors
then return `None`. It answers a real
suspension, resumes that same decoder with the selected action, then returns to
the driver before more native work. It keeps its report after a driver error
and is terminal after callback failure or caught unwind. `ContinueReading`
keeps affected output blocked; unavailable facts offer no valid action and
normally produce a traced refusal. Neither action certifies native workspace
or metadata disclosure safety.

### Native frame publication

`zencodec::frame` holds immutable interpretation and presentation facts shared
by policy and media sessions. `zencodec_media::frame` keeps compatible imports
of these same types. Native picture storage, mapping and sessions stay in
`zencodec-media`. These facts do not flatten planar samples into a packed
`zenpixels::PixelDescriptor`.

Media pump failures expose their upstream or downstream cause through
`Error::source()`. `MapError::Backend(Box<dyn Error + Send + Sync>)` keeps a
native mapping failure and its location trace inspectable through adapters and
type erasure. `Unavailable` has no backend cause. These debugging contracts are
part of the unpublished media surface; see [the media contract](media-contract.md#error-inspection-across-pumps-and-mappings).

```rust
// Canonical component code encoding (import directly, not re-exported):
use zenpixels::sample::SampleEncoding;
// zencodec::frame (public fields; non_exhaustive except Crop)
enum Subsampling { Mono, Yuv420, Yuv422, Yuv444 }
enum ChromaSiting { Unknown, Left, TopLeft, Center }
enum PlaneRole { Luma, Cb, Cr, Green, Blue, Red, Alpha }
struct Crop { x: u32, y: u32, width: u32, height: u32 }
struct FrameGeometry {
    coded_width: u32, coded_height: u32, crop: Crop,
    render: Option<(u32, u32)>,
}
struct ColorClaim {
    bitstream: Option<Cicp>, container: Option<Cicp>, resolved: Option<Cicp>,
}
struct FrameInterpretation {
    geometry: FrameGeometry, subsampling: Subsampling, siting: ChromaSiting,
    encoding: SampleEncoding, roles: Vec<PlaneRole>, color: ColorClaim,
    config_epoch: u32,
}
struct FrameTiming {
    pts: Option<Timestamp>, duration_ticks: Option<u64>,
    packet_ordinal: Option<u64>, input_offset: Option<u64>,
}
// zencodec::operation
enum PublicationOffer { ReturnHeld } // non_exhaustive
fn resolve_image<R: OperationPolicy + ?Sized>(
    image: image::ImageOutput, resolver: &mut R, preset: PolicySet, pulse: &dyn Pulse,
) -> Result<Outcome<image::ImageOutput>, At<PolicyFailure>>;
fn resolve_frame<R: OperationPolicy + ?Sized>(
    interpretation: &frame::FrameInterpretation, timing: frame::FrameTiming,
    resolver: &mut R, preset: PolicySet, pulse: &dyn Pulse,
) -> Result<DecisionRecord, At<PolicyFailure>>;

// zencodec_media::policy, implements VideoDecoder
struct PolicyDecoder<D, R> { /* private */ }
impl<D, R> PolicyDecoder<D, R> {
    fn new(decoder: D, resolver: R) -> Self;
    fn with_policy_set(self, preset: PolicySet) -> Self;
    fn report(&self) -> &OperationReport;
    const fn is_failed(&self) -> bool;
}
enum Stage { Decode, Policy } // non_exhaustive
struct Error { /* private: stage, report, native/traced cause */ }
impl Error {
    const fn stage(&self) -> Stage;
    fn report(&self) -> &OperationReport;
}
```

`resolve_image` consumes the held output and invokes the same resolver with
`Question::Image`. The callback can inspect immutable facts and map samples;
source names, profiles and typed extensions remain unfiltered data. Success
returns the same owner with its decision report. Refusal, cancellation, callback
failure, stale choice or unwind drops the unpublished owner. Caller-retained
clones remain independent.

`ImageFacts` records geometry, descriptor, declared plane/auxiliary counts,
applied retained spot contributions, source-extension count and optional native
frame/timing facts. These diagnostics contain no source payloads. Image selection
records retain these facts once in `DecisionContext::Image`; the selected offer
names only its effect. `PublicationOffer` is shared by image and frame questions,
replacing the unpublished frame-only option type. No additional resolver trait
or duplicate native-publication option enum is introduced.
`ReturnHeld` promises these held samples unchanged; it does not claim they are
the original samples before an explicit conversion.

Both publication routes follow native reconstruction. Presets requiring metadata
treatment, disallowing opaque profiles or requiring a workspace bound refuse
because native publication alone supplies no such proof. Spot counts describe
retained auxiliary channels; absence does not certify complete source discovery.
This boundary does not apply a spot plane again or transform pixels.

`FrameFacts` is the bounded report snapshot of geometry, subsampling, siting,
encoding, plane count, color, epoch and timing; the question borrows the full
component roles. `DecisionContext::Frame(FrameFacts)` remains available on
success and failure; `Selection::Frame(PublicationOffer)` identifies a successful
choice without duplicating the facts. Similarly,
`Selection::Boundary(BoundaryAction)` records only the mechanism, with the
interpretation in `DecisionContext::Boundary`. Consumers inspect one source of
facts for successful and refused operations.

Every native presentation passes policy before publication. The held frame and
its picture owner survive unchanged on success. Failure or unwind drops the
decoder and unpublished owners; later work returns `SessionError::Finished`.
Previously returned frames remain valid. `Error::source` preserves native
causes or `At<PolicyFailure>` and the original callback's `At<E>` chain.

The native decoder has already reconstructed the frame at this boundary. The
offer proves neither native allocation bounds nor container privacy. Presets
requiring workspace or disclosure evidence therefore refuse. A conversion or
encoder downstream must still check its own input contract. Frame and plane signatures use `zenpixels::sample::SampleEncoding` directly.
The duplicate core/media type and error are removed in the ablation candidate.
See [the migration boundary](precision-admission.md#shared-sample-description-migration-boundary-2026-10-30)
for migration and qualification scope.

[Native AV1 qualification](../perf/av1-policy-validation/README.md) exercises
the publication guard, native ownership and independent re-encode readback.
The guide separately sets the native `rav1d_safe::Settings::max_picture_bytes`
before constructing the decoder. That optional allowance admits requested
picture-plane bytes before allocation and follows retained native ownership.
It does not supply whole-decoder workspace evidence to a `PublicationOffer`; native
refusal remains a decode-stage error with its original trace.

## Encode traits

### `EncoderConfig` (codec config, `Clone + Send + Sync`)

```rust
trait EncoderConfig: Clone + Send + Sync {
    type Error: core::error::Error + Send + Sync + 'static;
    type Job: EncodeJob<Error = Self::Error>;

    fn format() -> ImageFormat;
    fn supported_descriptors() -> &'static [PixelDescriptor];
    fn capabilities() -> &'static EncodeCapabilities;    // default: EMPTY

    // Universal knobs (default no-op, codec overrides what it supports)
    fn with_generic_quality(self, quality: f32) -> Self;  // default: self
    fn with_generic_effort(self, effort: i32) -> Self;    // default: self
    fn with_lossless(self, lossless: bool) -> Self;       // default: self
    fn with_alpha_quality(self, quality: f32) -> Self;    // default: self
    fn with_prefer_streaming(self, prefer: bool) -> Self; // default: self
    fn generic_quality(&self) -> Option<f32>;   // default: None
    fn generic_effort(&self) -> Option<i32>;    // default: None
    fn is_lossless(&self) -> Option<bool>;      // default: None
    fn alpha_quality(&self) -> Option<f32>;     // default: None
    fn prefers_streaming(&self) -> Option<bool>; // default: None

    fn job(self) -> Self::Job;

    // Provided one-shot (default body; requires Job::Enc: Encoder<Error = Self::Error>):
    // job() → encoder() → encode(pixels) with default job settings.
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;
}
```

### `EncodeJob` (per-operation, owns SharedPulse and metadata)

```rust
trait EncodeJob: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    type Enc: Sized + 'static;                    // single-image encoder
    type AnimationFrameEnc: Sized + Send + 'static; // animation encoder

    fn with_pulse(self, pulse: SharedPulse) -> Self;
    fn with_limits(self, limits: ResourceLimits) -> Self;
    fn with_policy(self, policy: EncodePolicy) -> Self;         // default: self
    // Blessed metadata path: filters via Metadata::filtered(&policy) before the
    // codec sees the record, then routes through with_metadata. (default provided)
    fn with_metadata_policy(self, meta: Metadata, policy: MetadataPolicy) -> Self;
    #[deprecated] // embeds without a retention policy; use with_metadata_policy.
    fn with_metadata(self, meta: Metadata) -> Self; // primitive — codecs impl this
    fn with_canvas_size(self, width: u32, height: u32) -> Self; // default: self
    fn with_loop_count(self, count: Option<u32>) -> Self;       // default: self

    // Codec-specific extensions (downcasted by callers who know the codec)
    fn extensions(&self) -> Option<&dyn Any>;          // default: None
    fn extensions_mut(&mut self) -> Option<&mut dyn Any>; // default: None

    fn encoder(self) -> Result<Self::Enc, Self::Error>;
    fn animation_frame_encoder(self) -> Result<Self::AnimationFrameEnc, Self::Error>;
    // One-shot: encode with this job's config (== self.encoder()?.encode(pixels))
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>
        where Self::Enc: Encoder<Error = Self::Error>;

    // Type-erased convenience (default impls via shims)
    // Move a configured, retained job into shared dispatch before native startup.
    fn into_dyn_job(self) -> Box<dyn DynEncodeJob + Send>
    where Self: Send + 'static, Self::Enc: Encoder + Send,
          Self::AnimationFrameEnc: AnimationFrameEncoder;

    fn dyn_encoder(self) -> Result<Box<dyn DynEncoder>, BoxedError>
        where Self::Enc: Encoder;
    fn dyn_animation_frame_encoder(self) -> Result<Box<dyn DynAnimationFrameEncoder>, BoxedError>
        where Self::AnimationFrameEnc: AnimationFrameEncoder;
}
```

### `Encoder` (type-erased single-image encode)

```rust
trait Encoder: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    fn reject(op: UnsupportedOperation) -> Self::Error;
    fn preferred_strip_height(&self) -> u32;    // default: 0
    fn encode(self, pixels: PixelSlice<'_>) -> Result<EncodeOutput, Self::Error>;

    // Hot path: encode from mutable sRGB RGBA8 buffer (encoder may modify in-place)
    fn encode_srgba8(
        self, data: &mut [u8], make_opaque: bool,
        width: u32, height: u32, stride_pixels: u32,
    ) -> Result<EncodeOutput, Self::Error>;  // default: wraps encode()

    // Row-level push (mutually exclusive with encode)
    fn begin(&mut self, width: u32, height: u32, descriptor: PixelDescriptor)
        -> Result<OutputTiming, Self::Error>;                                   // default: Err(RowLevelEncode)
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), Self::Error>;  // default: Err
    fn pending_output(&self) -> &[u8];                                          // default: empty
    fn consume_output(&mut self, n: usize);                                     // default: no-op
    fn take_output(&mut self, into: &mut Vec<u8>);                              // default: copy + consume
    fn finish(self) -> Result<EncodeOutput, Self::Error>;                       // default: Err

    // Declared, stepwise pull. InputRequirement::Replay selects this protocol.
    fn pull_rows(&mut self, rows: &mut dyn EncodeRows, pulse: &dyn Pulse)
        -> Result<PullProgress, Self::Error>; // default: Err(PullEncode), no source call

    // Legacy one-call pull from source callback
    fn encode_from(
        self, source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize,
    ) -> Result<EncodeOutput, Self::Error>;  // default: Err
}
```

Mutually exclusive paths: `encode()`/`encode_srgba8()`, `push_rows()+finish()`,
legacy `encode_from()`, or declared `pull_rows()+finish_output()`.

`begin` is optional before `push_rows`. It declares the image and returns
an `OutputTiming`: when *this* encoder's bytes arrive, given its
configuration, image and metadata. `EncodeCapabilities::streaming_output`
says the codec can stream in some configuration; `begin` says whether it
will. An encoder whose configuration cannot encode from rows rejects
`begin` with `UnsupportedOperation::RowLevelEncode`, before any row moves.

With `OutputTiming::streaming(lag_rows)` the encoder writes its header at
`begin` and exposes encoded bytes through `pending_output` as rows arrive,
holding back at most `lag_rows` pushed rows; the caller sends them and calls
`consume_output`, and `finish` returns only what was not consumed. With
`OutputTiming::AT_FINISH` `pending_output` stays empty. Draining never
changes the bytes. Pushing more or fewer rows than declared is an error.

Rules (spelled out on `Encoder::begin`):

- `begin` answers as `encode()` of that size would. A second `begin` is an
  error, and a failed `begin` leaves the encoder unchanged.
- `encode()`/`encode_from()` after `begin` or `push_rows` is an error.
- The lag is below the image height and counts pushed rows whose bytes
  are not yet pending. A coder without row-aligned output flushes on a row
  schedule fixed at `begin`.
- `begin` may choose framing (flush points, chunking) that makes the file
  differ from `encode()`'s; it may never change the pixels.
- `consume_output(n)` with `n` above the pending length consumes all of it.
  `take_output(into)` appends the pending bytes to `into` and consumes
  them; an encoder holding its output in `OutputBuffer` swaps allocations
  when `into` is empty, so nothing is copied.
- After an error from `push_rows` or `finish` the encode cannot complete;
  a driver that has sent bytes aborts its transport.
- `max_output_bytes` bounds the whole file, including bytes already handed
  out.

`EncoderConfig::with_prefer_streaming(true)` asks, codec-agnostically, for
a configuration that streams (JPEG: fixed tables, sequential; PNG: no
whole-image reductions). Explicit codec settings win; `begin` reports what
the encoder will do.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct OutputTiming { /* private */ }               // Default = AT_FINISH
impl OutputTiming {
    const AT_FINISH: Self;                         // everything from finish (the default)
    const fn streaming(lag_rows: u32) -> Self;     // bytes stream; at most lag_rows held
    const fn streams(&self) -> bool;
    const fn lag_rows(&self) -> Option<u32>;       // None when AT_FINISH
}
```

`OutputBuffer` is the output side of a streaming encoder, shared so codecs
do not each write it: `push`, `vec_mut` (append-only access for writers
that need a `Vec<u8>`), `pending`, `consume` (clamps; drops the sent prefix
once it is at least 64 KiB and half the buffer), `take_into` (swaps when the
destination is empty), `handed_out`, `total` (what `max_output_bytes`
bounds), `try_reserve`, and `into_remaining` (what `finish` returns).

### `AnimationFrameEncoder` (animation encode)

```rust
trait AnimationFrameEncoder: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    fn reject(op: UnsupportedOperation) -> Self::Error;
    fn begin(&mut self, width: u32, height: u32, descriptor: PixelDescriptor,
        frame_count: Option<u32>) -> Result<AnimationOutputTiming, Self::Error>; // default: Ok(AT_FINISH)
    fn pending_output(&self) -> &[u8];                     // default: empty
    fn consume_output(&mut self, n: usize);                // default: no-op
    fn take_output(&mut self, into: &mut Vec<u8>);         // default: copy + consume
    fn push_frame(
        &mut self, pixels: PixelSlice<'_>, duration_ms: u32, pulse: Option<&dyn Pulse>,
    ) -> Result<(), Self::Error>;
    fn push_frame_timed(&mut self, pixels: PixelSlice<'_>, duration: FrameDuration,
        pulse: Option<&dyn Pulse>) -> Result<FrameDuration, Self::Error>;   // default: exact
    fn push_frame_exact(&mut self, pixels: PixelSlice<'_>, duration: FrameDuration,
        pulse: Option<&dyn Pulse>) -> Result<(), Self::Error>;              // default: Err
    fn finish(self, pulse: Option<&dyn Pulse>) -> Result<EncodeOutput, Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct AnimationOutputTiming { /* private */ }      // Default = AT_FINISH
impl AnimationOutputTiming {
    const AT_FINISH: Self;
    const fn streaming(lag_frames: u32) -> Self;
    const fn streams(&self) -> bool;
    const fn lag_frames(&self) -> Option<u32>;
}
```

`begin` is the animation counterpart of `Encoder::begin`, counted in
frames: it declares the canvas, the frame descriptor and, when known, the
frame count, before the first frame. A streaming encoder writes its header
then and each frame's bytes as frames are pushed, holding back at most
`lag_frames` frames; `finish` returns what was not consumed. A format that
writes the frame count first (APNG) streams only when it is given;
finishing after a different number of declared frames is an error. A
second `begin` is an error and a failed one changes nothing. Draining never
changes the bytes, and `begin` may change framing, never pixels or timing.

Full-canvas frames only. Animation encoder is `'static` — it owns its data.
Codecs without animation set `type AnimationFrameEnc = ()` (unit implements
`AnimationFrameEncoder` with all methods returning `Err`).

---

## Decode traits

### `DecoderConfig` (codec config, `Clone + Send + Sync`)

```rust
trait DecoderConfig: Clone + Send + Sync {
    type Error: core::error::Error + Send + Sync + 'static;
    type Job<'a>: DecodeJob<'a, Error = Self::Error> + 'static;

    fn formats() -> &'static [ImageFormat]; // may return multiple
    fn supported_descriptors() -> &'static [PixelDescriptor];
    fn capabilities() -> &'static DecodeCapabilities;  // default: EMPTY

    fn job<'a>(self) -> Self::Job<'a>;

    // Provided one-shots (default bodies) with default job settings:
    fn decode(self, data: &[u8]) -> Result<DecodeOutput, Self::Error>;  // native pixel format
    fn probe(&self, data: &[u8]) -> Result<ImageInfo, Self::Error>;     // header parse only
}
```

### `DecodeJob<'a>` (per-operation, holds limits/SharedPulse/hints)

```rust
trait DecodeJob<'a>: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    type Dec: Decode<Error = Self::Error>;
    type StreamDec: StreamingDecode<Error = Self::Error>;
    type AnimationFrameDec: AnimationFrameDecoder<Error = Self::Error> + 'static;

    fn with_pulse(self, pulse: SharedPulse) -> Self;
    fn with_limits(self, limits: ResourceLimits) -> Self;
    fn with_policy(self, policy: DecodePolicy) -> Self;  // default: self

    // Probing (needs limits + Pulse context)
    fn probe(&self, data: &[u8]) -> Result<ImageInfo, Self::Error>;      // header only
    fn probe_full(&self, data: &[u8]) -> Result<ImageInfo, Self::Error>; // default: probe()

    // Decode hints (optional, decoder may ignore)
    fn with_crop_hint(self, x: u32, y: u32, width: u32, height: u32) -> Self;  // default: self
    fn with_orientation(self, hint: OrientationHint) -> Self;                   // default: self
    fn with_start_frame_index(self, index: u32) -> Self;                        // default: self

    // Codec-specific extensions
    fn extensions(&self) -> Option<&dyn Any>;          // default: None
    fn extensions_mut(&mut self) -> Option<&mut dyn Any>; // default: None

    // Output prediction
    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, Self::Error>;

    // Executor creation — all bind data + preferred here
    // data is Cow<'a, [u8]> — pass Cow::Borrowed for zero-copy, Cow::Owned to donate
    fn decoder(self, data: Cow<'a, [u8]>, preferred: &[PixelDescriptor])
        -> Result<Self::Dec, Self::Error>;
    fn push_decoder(self, data: Cow<'a, [u8]>, sink: &mut dyn DecodeRowSink,
        preferred: &[PixelDescriptor]) -> Result<OutputInfo, Self::Error>;
    fn streaming_decoder(self, data: Cow<'a, [u8]>, preferred: &[PixelDescriptor])
        -> Result<Self::StreamDec, Self::Error>;
    fn animation_frame_decoder(self, data: Cow<'a, [u8]>, preferred: &[PixelDescriptor])
        -> Result<Self::AnimationFrameDec, Self::Error>;

    // Type-erased convenience (default impls via shims)
    fn dyn_decoder(...) -> Result<Box<dyn DynDecoder + 'a>, BoxedError>;
    fn dyn_animation_frame_decoder(...) -> Result<Box<dyn DynAnimationFrameDecoder>, BoxedError>;
    fn dyn_streaming_decoder(...) -> Result<Box<dyn DynStreamingDecoder + 'a>, BoxedError>;
}
```

`preferred` is a ranked list of desired output formats — the decoder picks the
first it can produce without lossy conversion. Pass `&[]` for native format.

### `Decode` (single-image decode, returns owned pixels)

```rust
trait Decode: Sized {
    type Error: core::error::Error + Send + Sync + 'static;
    fn prepare_output(&mut self) -> Result<Option<PixelDescriptor>, Self::Error>; // default None
    fn decode(self) -> Result<DecodeOutput, Self::Error>;
}
```

`DecodeRequest::with_output` uses this optional preparation automatically on the
same configured decoder that will reconstruct the image. Callers need no separate
capability query. `None` means the operation must decode and validate its actual
result; it does not mean unsupported decoding. A proven mismatch can refuse before
raster reconstruction, and successful output is still checked afterwards.

Implementors may inspect input and retain metadata, but preparation must not
reconstruct pixels or coefficients or allocate an output raster. Retained Pulse
controls still apply. Native errors remain primary; after successful preparation,
the operation checks cancellation before comparing descriptors. The default does
no work. Native JPEG adoption and measured costs are scoped separately in
[descriptor admission](decode-descriptor-admission.md).

### `StreamingDecode` (scanline-batch decode, pull iterator)

```rust
trait StreamingDecode {
    type Error: core::error::Error + Send + Sync + 'static;
    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, Self::Error>;
    fn info(&self) -> &ImageInfo;
}
```

`impl StreamingDecode for ()` is the rejection stub — set `type StreamDec = ()`
for codecs that don't support streaming.

### `AnimationFrameDecoder` (animation decode, composited full-canvas frames)

```rust
trait AnimationFrameDecoder: Sized {
    type Error: core::error::Error + Send + Sync + 'static;

    fn wrap_sink_error(err: SinkError) -> Self::Error;
    fn info(&self) -> &ImageInfo;
    fn frame_count(&self) -> Option<u32>;       // default: None
    fn loop_count(&self) -> Option<u32>;        // default: None

    fn render_next_frame(&mut self, pulse: Option<&dyn Pulse>)
        -> Result<Option<AnimationFrame<'_>>, Self::Error>;
    fn render_next_frame_owned(&mut self, pulse: Option<&dyn Pulse>)
        -> Result<Option<OwnedAnimationFrame>, Self::Error>;   // default: copies from render_next_frame
    fn render_next_frame_to_sink(&mut self, pulse: Option<&dyn Pulse>,
        sink: &mut dyn DecodeRowSink) -> Result<Option<OutputInfo>, Self::Error>;
}
```

Use `Unsupported<E>` as the associated type for codecs without animation support.

### `IncrementalDecode` (resumable sans-I/O decode)

```rust
trait IncrementalDecode {
    type Error: core::error::Error + Send + Sync + 'static;
    fn step(&mut self, input: Input<'_>, sink: &mut dyn DecodeRowSink, pulse: &dyn Pulse)
        -> Result<Progress, StepError<Self::Error>>;
    fn requested(&self) -> &[InputRequest];                         // after Event::Need, never empty
    fn info(&self) -> Option<&ImageInfo>;
    fn output_descriptor(&self) -> Option<PixelDescriptor>;   // default None
    fn access_pattern(&self) -> source::AccessPattern;         // default Sequential
}

struct Input<'a> { offset: u64, bytes: &'a [u8], total_len: Option<u64> }
struct Progress { consumed: usize, event: Event }
#[non_exhaustive] enum Event { Need, Rows { meta: BatchMeta, rows: u32 }, Metadata(MetadataUpdate), Done(OutputInfo) }
#[non_exhaustive] enum InputRequest { AtLeast { offset: u64, min_len: usize }, Range(ByteRange), Len }
#[non_exhaustive] enum StepError<E> { Codec(E), Sink(SinkError), Cancelled(StopReason), Finished }

struct Fused<D>;                                   // enforces Done/error/stop termination
fn erase_incremental<D>(d: D) -> Box<DynIncrementalDecoder>;  // Fused + type erasure
type DynIncrementalDecoder = dyn IncrementalDecode<Error = ErasedError> + Send;
```

The driver owns the input: each `step` is offered bytes at an absolute
offset, takes a prefix (`consumed`) and reports one event. After `Need`, the
driver satisfies any one of `requested()`. Rows go straight into the sink;
final rows once each through `provide_next_buffer` in increasing `y`.
Errors and cancellation are terminal. Created by
`DecodeJob::dyn_incremental_decoder` (returns `Box<DynIncrementalDecoder>`,
built with `erase_incremental`); declared with
`DecodeCapabilities::incremental`. Drivers (slice, `ReadAt`,
`std::io::Read`, async) are in `zencodec-io`; straight-line authoring is in
`zencodec-coroutine`. Rules, drivers and measurements:
[`range-decoding-contract.md`](range-decoding-contract.md).

### `ReadAt` / `DynReadAt` (random-access source) and `push_decoder_from_source`

```rust
// zencodec::source
trait ReadAt {
    type Error: core::error::Error + Send + Sync + 'static;
    fn len(&mut self) -> Result<u64, Self::Error>;            // may read a stream to its end
    fn len_hint(&self) -> Option<u64>;                        // default None
    fn read_exact_at(&mut self, offset: u64, out: &mut [u8]) -> Result<(), Self::Error>;
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize, Self::Error>; // short only at the end
    fn read_many_at(&mut self, reqs: &mut [(u64, &mut [u8])]) -> Result<(), Self::Error>; // default: sequential
    fn as_slice(&self) -> Option<&[u8]>;                      // default None; in-memory sources
}
trait DynReadAt { /* same, over BoxedError */ }
struct SliceReadAt<'a>;  struct ReadAtShim<S>(pub S);
// Box<dyn DynReadAt + 'a>: local or borrowed source.
// Box<dyn DynReadAt + Send + 'a>: explicitly transferable source.
enum AccessPattern { Sequential, BoundedBacktrack { window_bytes: u64 }, RandomAccess }
enum SourceError { BeyondWindow {..}, BeyondLimit {..}, UnexpectedEof {..}, Io {..} }

// zencodec-io, feature std: filesystem/reader errors retain their native cause.
enum StdSourceError { Source(SourceError), Io(std::io::Error), Allocation { requested: usize, source: TryReserveError } }
// FileReadAt and BufferedReadAt implement ReadAt<Error = StdSourceError>.
struct ReadAtReader<S> { /* independent logical cursor; no internal buffer */ }
impl<S> ReadAtReader<S> {
    const fn new(source: S) -> Self;
    fn into_inner(self) -> S;
}
// impl<S: ReadAt> std::io::{Read, Seek} for ReadAtReader<S>.
// End-relative seeks call len(); other seeks do no I/O. Failed operations do
// not advance the logical cursor, but the source/buffer may have changed.
// Native io::Error and StdSourceError::Io values survive unchanged. Other
// source errors remain typed payloads with an underlying I/O kind, when present.
// BufferPolicy::try_for_pattern(pattern, max_bytes) -> Result<BufferPolicy, SourceError>
// rejects an undersized required backtrack window before constructing a source.

// zencodec-io: retained driver input admission, shared by sync/async drivers.
struct IoLimits { /* private; new(input_bytes, ranges, in_flight), unlimited() */ }
impl DriveOptions {
    fn with_limits(self, limits: IoLimits) -> Self;
    fn with_work_quota(self, quota: WorkQuota) -> Self;
}
struct WorkQuota { /* private; new(nonzero_steps) -> Option<Self>, unlimited() */ }
// Async compute/packet scheduling only: yielding does not drain output.
enum InputError {
    Limit { needed: u64, limit: usize }, RangesDisabled, ReadsDisabled,
    Allocation { bytes: usize, source: TryReserveError },
}
// DriveFailure::Input / AsyncDriveError::Input retain these errors separately
// from source and codec failures. Zero allowances still permit borrowed slices.
// Ranges are alternatives; optional prefetch stays within the allowance.
// Capacity excludes sources, codecs, output and allocator transients.

// DecodeJob<'a> — default Err(UnsupportedOperation::SeekableDecode)
fn push_decoder_from_source(self, source: &mut dyn DynReadAt,
    sink: &mut dyn DecodeRowSink, preferred: &[PixelDescriptor])
    -> Result<OutputInfo, BoxedError>;
```

`push_decoder` with the data binding changed from a slice to a source: the
codec reads exactly the ranges it needs in straight-line code and writes
rows into the sink. Declared with `DecodeCapabilities::seekable`. Also on
`DynDecodeJob` and as `CodecSet::push_decode_from_source` (probes offset 0,
then dispatches).

### `DecodeRowSink` (zero-copy row sink, push-based)

```rust
trait DecodeRowSink {
    fn begin(&mut self, width: u32, height: u32, descriptor: PixelDescriptor)
        -> Result<(), SinkError>;  // default: Ok(())
    fn provide_next_buffer(&mut self, y: u32, height: u32, width: u32,
        descriptor: PixelDescriptor) -> Result<PixelSliceMut<'_>, SinkError>;
    fn finish(&mut self) -> Result<(), SinkError>;  // default: Ok(())
    fn accepts_provisional(&self) -> bool;          // default: false
    fn provide_pass_buffer(&mut self, meta: BatchMeta, height: u32, width: u32,
        descriptor: PixelDescriptor) -> Result<PixelSliceMut<'_>, SinkError>; // default: Err
}
```

The codec calls `begin()`, then `provide_next_buffer()` per strip, writes
decoded pixels via `PixelSliceMut::row_mut()`, then calls `finish()`. The sink
controls stride (can return SIMD-aligned buffers). Object-safe.

Final rows arrive once each through `provide_next_buffer`, in increasing
`y`, for every sink. A progressive decoder also writes provisional rows
through `provide_pass_buffer`, only when `accepts_provisional()` (read once,
at `begin`) is `true`.

`SinkError = Box<dyn core::error::Error + Send + Sync>`

---

## Dyn dispatch traits

### Encode side

```rust
trait DynEncoderConfig: Send + Sync {
    fn as_any(&self) -> &dyn Any;  // downcast to concrete config
    fn format(&self) -> ImageFormat;
    fn supported_descriptors(&self) -> &'static [PixelDescriptor];
    fn capabilities(&self) -> &'static EncodeCapabilities;
    fn dyn_job(&self) -> Box<dyn DynEncodeJob + 'static>;
}

trait DynEncodeJob {
    fn set_pulse(&mut self, pulse: SharedPulse);
    fn set_limits(&mut self, limits: ResourceLimits);
    fn set_policy(&mut self, policy: EncodePolicy);
    fn set_metadata_policy(&mut self, meta: Metadata, policy: MetadataPolicy); // blessed
    #[deprecated] // use set_metadata_policy
    fn set_metadata(&mut self, meta: Metadata);
    fn set_canvas_size(&mut self, width: u32, height: u32);
    fn set_loop_count(&mut self, count: Option<u32>);
    fn extensions(&self) -> Option<&dyn Any>;
    fn extensions_mut(&mut self) -> Option<&mut dyn Any>;
    // One-shot: encode with this job's config (== self.into_encoder()?.encode(pixels))
    fn encode(self: Box<Self>, pixels: PixelSlice<'_>) -> Result<EncodeOutput, BoxedError>;
    fn into_encoder(self: Box<Self>) -> Result<Box<dyn DynEncoder>, BoxedError>;
    fn into_animation_frame_encoder(self: Box<Self>) -> Result<Box<dyn DynAnimationFrameEncoder>, BoxedError>;
}

trait DynEncoder {
    fn preferred_strip_height(&self) -> u32;
    fn encode(self: Box<Self>, pixels: PixelSlice<'_>) -> Result<EncodeOutput, BoxedError>;
    fn encode_srgba8(self: Box<Self>, data: &mut [u8], make_opaque: bool,
        width: u32, height: u32, stride_pixels: u32) -> Result<EncodeOutput, BoxedError>;
    fn begin(&mut self, width: u32, height: u32, descriptor: PixelDescriptor) -> Result<OutputTiming, BoxedError>;
    fn push_rows(&mut self, rows: PixelSlice<'_>) -> Result<(), BoxedError>;
    fn pending_output(&self) -> &[u8];
    fn consume_output(&mut self, n: usize);
    fn take_output(&mut self, into: &mut Vec<u8>);
    fn finish(self: Box<Self>) -> Result<EncodeOutput, BoxedError>;
    fn encode_from(self: Box<Self>,
        source: &mut dyn FnMut(u32, PixelSliceMut<'_>) -> usize) -> Result<EncodeOutput, BoxedError>;
}

trait DynAnimationFrameEncoder {
    fn as_any(&self) -> &dyn Any;       // downcast to concrete encoder
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
    fn begin(&mut self, width: u32, height: u32, descriptor: PixelDescriptor,
        frame_count: Option<u32>) -> Result<AnimationOutputTiming, BoxedError>;
    fn pending_output(&self) -> &[u8];
    fn consume_output(&mut self, n: usize);
    fn take_output(&mut self, into: &mut Vec<u8>);
    fn push_frame(&mut self, pixels: PixelSlice<'_>, duration_ms: u32,
        pulse: Option<&dyn Pulse>) -> Result<(), BoxedError>;
    fn push_frame_timed(&mut self, pixels: PixelSlice<'_>, duration: FrameDuration,
        pulse: Option<&dyn Pulse>) -> Result<FrameDuration, BoxedError>;
    fn push_frame_exact(&mut self, pixels: PixelSlice<'_>, duration: FrameDuration,
        pulse: Option<&dyn Pulse>) -> Result<(), BoxedError>;
    fn finish(self: Box<Self>, pulse: Option<&dyn Pulse>) -> Result<EncodeOutput, BoxedError>;
}
```

### Decode side

```rust
trait DynDecoderConfig: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn formats(&self) -> &'static [ImageFormat];
    fn supported_descriptors(&self) -> &'static [PixelDescriptor];
    fn capabilities(&self) -> &'static DecodeCapabilities;
    fn dyn_job(&self) -> Box<dyn DynDecodeJob<'_> + '_>;
}

trait DynDecodeJob<'a> {
    fn set_pulse(&mut self, pulse: SharedPulse);
    fn set_limits(&mut self, limits: ResourceLimits);
    fn set_policy(&mut self, policy: DecodePolicy);
    fn probe(&self, data: &[u8]) -> Result<ImageInfo, BoxedError>;
    fn probe_full(&self, data: &[u8]) -> Result<ImageInfo, BoxedError>;
    fn set_crop_hint(&mut self, x: u32, y: u32, width: u32, height: u32);
    fn set_orientation(&mut self, hint: OrientationHint);
    fn set_start_frame_index(&mut self, index: u32);
    fn extensions(&self) -> Option<&dyn Any>;
    fn extensions_mut(&mut self) -> Option<&mut dyn Any>;
    fn output_info(&self, data: &[u8]) -> Result<OutputInfo, BoxedError>;
    fn into_decoder(self: Box<Self>, data: Cow<'a, [u8]>, preferred: &[PixelDescriptor])
        -> Result<Box<dyn DynDecoder + 'a>, BoxedError>;
    fn push_decode(self: Box<Self>, data: Cow<'a, [u8]>,
        sink: &mut dyn DecodeRowSink, preferred: &[PixelDescriptor])
        -> Result<OutputInfo, BoxedError>;
    fn into_streaming_decoder(self: Box<Self>, data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor])
        -> Result<Box<dyn DynStreamingDecoder + 'a>, BoxedError>;
    fn into_animation_frame_decoder(self: Box<Self>, data: Cow<'a, [u8]>,
        preferred: &[PixelDescriptor])
        -> Result<Box<dyn DynAnimationFrameDecoder>, BoxedError>;
}

trait DynDecoder {
    fn prepare_output(&mut self) -> Result<Option<PixelDescriptor>, BoxedError>; // default None
    fn decode(self: Box<Self>) -> Result<DecodeOutput, BoxedError>;
}

trait DynAnimationFrameDecoder {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
    fn info(&self) -> &ImageInfo;
    fn frame_count(&self) -> Option<u32>;
    fn loop_count(&self) -> Option<u32>;
    fn render_next_frame_owned(&mut self, pulse: Option<&dyn Pulse>)
        -> Result<Option<OwnedAnimationFrame>, BoxedError>;
    fn render_next_frame_to_sink(&mut self, pulse: Option<&dyn Pulse>,
        sink: &mut dyn DecodeRowSink) -> Result<Option<OutputInfo>, BoxedError>;
}

trait DynStreamingDecoder {
    fn next_batch(&mut self) -> Result<Option<(u32, PixelSlice<'_>)>, BoxedError>;
    fn info(&self) -> &ImageInfo;
}

// The erased incremental decoder is the trait object itself:
type DynIncrementalDecoder = dyn IncrementalDecode<Error = ErasedError> + Send;
// ErasedError wraps BoxedError; its source() is the codec's error.
```

### Downcasting rules

- `DynEncoderConfig`, `DynDecoderConfig`: `as_any()` — configs are `'static`
- `DynAnimationFrameEncoder`, `DynAnimationFrameDecoder`: `as_any()`, `as_any_mut()`, `into_any()` — frame decoders/encoders are `'static`
- `DynEncoder`, `DynDecoder`, `DynStreamingDecoder`: **no downcasting** — they borrow `'a` data

Use `extensions()`/`extensions_mut()` on jobs for codec-specific access through the dyn pipeline.

---

## Pixel types (from `zenpixels`)

These types are defined in `zenpixels` and used throughout the zen ecosystem.
All crates depend on `zenpixels` directly. See `zenpixels` documentation.

Key types: `PixelSlice<'a>`, `PixelSliceMut<'a>`, `PixelBuffer`, `PixelDescriptor`,
`PixelFormat`, `ChannelLayout`, `ChannelType`, `SignalRange`, `TransferFunction`,
`ColorPrimaries`, `AlphaMode`.

---

## Image metadata

### `ImageInfo`

Image metadata from probing or decoding. `#[non_exhaustive]`, `Clone + Debug + PartialEq`.

Fields: `width`, `height`, `format: ImageFormat`, `has_alpha`,
`sequence: ImageSequence` (`Single`, `Animation { frame_count, loop_count,
random_access }`, `Multi { image_count, random_access }`; `is_animation()` and
`frame_count()` read it), `orientation: Orientation`,
`source_color: SourceColor`, `embedded_metadata: EmbeddedMetadata`,
`has_gain_map`,
`source_encoding: Option<Arc<dyn SourceEncodingDetails>>`,
`warnings: Vec<String>`.

Builder pattern: `ImageInfo::new(w, h, format).with_alpha(true).with_cicp(...)`.

Key methods: `display_width()`, `display_height()` (orientation-corrected),
`transfer_function()`, `color_primaries()`,
`metadata() -> Metadata`,
`source_encoding_details() -> Option<&dyn SourceEncodingDetails>`.

`PartialEq` skips `source_encoding` (trait objects aren't comparable).

### `SourceColor`

Source color description. Fields: `cicp: Option<Cicp>`,
`icc_profile: Option<Arc<[u8]>>`, `bit_depth: Option<u8>`,
`channel_count: Option<u8>`, `content_light_level: Option<ContentLightLevel>`,
`mastering_display: Option<MasteringDisplay>`.

Builder pattern: `SourceColor::default().with_cicp(...).with_icc_profile(...)`.

Methods: `transfer_function()`, `color_primaries()`.

Note: sRGB detection (`is_srgb()`) lives in zencodecs as `SourceColorExt`,
not here — zencodec stores color metadata but doesn't classify it.

### `EmbeddedMetadata`

Non-color metadata blobs. Fields: `exif: Option<Vec<u8>>`, `xmp: Option<Vec<u8>>`.

### `Metadata`

Owned metadata for encode/decode roundtrip. Fields: `icc_profile`, `exif`, `xmp`
(`Option<Arc<[u8]>>`), `cicp`, `content_light_level`, `mastering_display` (Copy),
and `orientation`. `#[non_exhaustive]`. Carries no retention state — a policy is
chosen *transiently* at embed time (see below), not stored here.

Methods: builder pattern (`with_icc()`, `with_exif()`, `with_xmp()`, etc.),
`with_copyright(&str)` / `with_artist(&str)` (build-or-merge the rights tag into
the EXIF blob, ASCII), `transfer_function()`, `color_primaries()`, `is_empty()`,
`filtered(&MetadataPolicy) -> Metadata`. `From<&ImageInfo>` conversion.

**Embed-time policy (explicit privacy decision, compile-time enforced).**
Retention is decided when metadata is handed to the encoder, via
`EncodeJob::with_metadata_policy(meta, policy)` (or
`DynEncodeJob::set_metadata_policy`), which applies `meta.filtered(&policy)`
*before* the record reaches the codec — so a codec only ever embeds what the
policy kept. The plain `with_metadata` / `set_metadata` are `#[deprecated]`:
they embed without a retention choice, so the compiler **warns at the call site**
(a nudge, not a semver break — they still work, and codecs still *implement*
`with_metadata` as the primitive `with_metadata_policy` routes through;
deprecation warns callers, not implementors). The raw `exif`/`xmp`/`icc_profile`
bytes stay untouched until that filter runs, so an inspect / bring-your-own
EXIF-library round-trip still sees the originals. `MetadataPolicy` has **no
`Default`** — name one explicitly; `Web` is the recommended privacy-safe choice,
`PreserveExact` keeps fields in the filtered record (with the documented EXIF
orientation reconciliation). These setters remain best-effort: the codec can
omit unsupported carriers. They do not provide checked retention admission.
See [the native retention witness](metadata-retention-admission.md).

### `MetadataPolicy` / `MetadataFields` / `IccRetention`

Field-level retention policy for `Metadata::filtered()` — the shared metadata
filter for re-encode / recompress pipelines.

`MetadataPolicy` (`#[non_exhaustive]`, **no `Default`** — retention is a privacy
decision the caller must make explicitly; `Web` is the recommended choice):
- `PreserveExact` — keep everything, byte-faithfully (incl. a redundant sRGB ICC).
- `Preserve` — keep everything, but drop a redundant sRGB ICC.
- `Web` (recommended) — ICC (unless redundant sRGB) + EXIF orientation/rights +
  CICP/HDR; drop the rest of EXIF (GPS, timestamps, camera, thumbnail) and XMP.
- `ColorAndRotation` — only what places pixels: ICC (non-sRGB) + CICP/HDR +
  EXIF orientation. Drops attribution, XMP, other EXIF.
- `Custom(MetadataFields)` — explicit per-field control.

`MetadataFields` (`Copy`, `#[non_exhaustive]`, `with_*` builders + `KEEP_ALL` /
`DISCARD_ALL` consts): `icc: IccRetention`, `exif: ExifPolicy`, and `xmp` /
`cicp` / `hdr: Retention`. `MetadataPolicy::fields()` resolves a policy.

`IccRetention`: `Drop` / `KeepNonSrgb` (drop only a redundant sRGB,
`zenpixels::icc::is_common_srgb`) / `Keep` (byte-faithful).

CICP / HDR are color *signaling* (dropping them changes displayed pixels), so
the presets keep them; only a `Custom` policy can drop them. Gain maps are not
part of `Metadata` (they live at the encode-request layer) and are unaffected.

### `exif::Exif` / `ExifPolicy` / `Retention`

Structured EXIF model (`zencodec::exif`). `Exif<'a>` (`parse` **or** `new` →
`filtered` / edit → `to_bytes`) borrows the source — entry values and the
thumbnail are never copied (entry values are `Cow`, borrowed on parse, owned when
injected by an edit). `Exif::new(TextEncoding)` (and `Default`, which uses `Ascii`)
starts an empty little-endian tree for building from scratch — e.g. stamp a
Copyright on an image that had no EXIF: `Exif::new(TextEncoding::Ascii)` →
`set_copyright(…)` → `to_bytes()` (raw TIFF; the codec adds the APP1 `Exif\0\0`
framing). The `TextEncoding` is **required** at `new` — it's the Exif 2.x ASCII
(type 2) vs Exif 3.0 UTF-8 (type 129) compat choice, a blob property used by all
string writes (type 129 is read by almost nothing today, so it can't be a silent
default). Read accessors: `orientation()`, `copyright()` / `artist()` (lossy-UTF-8
text *view*, borrowing `&self`), `copyright_bytes()` / `artist_bytes()` (raw
field bytes), `has_thumbnail()`, `has_gps()`. Edit accessors: `set_copyright(&str)`
/ `set_artist(&str)` insert-or-replace the IFD0 tag using the blob's `TextEncoding`
(materialized on the next `to_bytes`); `set_orientation(Orientation)`
insert-or-replaces the Orientation tag (an existing SHORT/LONG entry keeps its
TIFF type; a malformed non-integer carrier is replaced by the canonical 1-count
SHORT; the serializer writes IFDs tag-sorted, so insertion order is
immaterial). `to_bytes()` re-serializes a valid TIFF
with recomputed offsets, preserving byte order and `Exif\0\0` framing; it is a
byte-exact fixpoint, so filtering and editing stay idempotent.

`TextEncoding` (`#[non_exhaustive]`) — the EXIF text convention a write uses:
`Ascii` (Exif 2.x, TIFF type 2; carries UTF-8 bytes de-facto — most compatible,
the recommended default) or `Utf8` (Exif 3.0 / CIPA DC-008-2023, TIFF type 129;
spec-conformant Unicode, thin reader support). Both write the same UTF-8 bytes,
NUL-terminated; they differ only in the declared TIFF type. Re-exported at the
crate root.

Encoding (read side): Copyright/Artist may be ASCII (type 2, 7-bit) **or UTF-8
(type 129, Exif 3.0)**; non-ASCII bytes stuffed into a type-2 field are the
non-conformant-but-common case. zencodec reads both — `copyright` / `artist`
give a lossy-UTF-8 display view, `*_bytes` give the exact bytes. A pruning
rewrite **never transcodes**: it preserves the value bytes **and TIFF type**
verbatim (a field is neither corrupted nor "corrected"). Writing is the only
path that mints new bytes, and the caller picks the type via `TextEncoding`.

`ExifPolicy` (`Copy`, `#[non_exhaustive]`, `with_*` builders) — seven keep/drop
categories of `Retention`: `orientation`, `rights` (copyright + artist),
`thumbnail`, `gps`, `datetimes`, `camera`, `other`. Consts: `KEEP_ALL`,
`DISCARD_ALL`, `ATTRIBUTED_ORIENTATION`, `ORIENTATION_ONLY`.

`Retention` (`Keep` / `Discard`) — explicit per-field intent.

`exif::retain(&[u8], &ExifPolicy) -> Option<Cow<[u8]>>` — `Cow::Borrowed` when
nothing is dropped (so `Metadata::filtered` is a cheap `Arc` clone),
`Cow::Owned` on a rewrite, `None` when all EXIF is discarded.

`helpers::parse_exif_orientation` is a lightweight orientation accessor that
delegates here. Limitation: a partial rewrite that *keeps* `MakerNote` (0x927C)
relocates it without fixing its maker-specific internal offsets — keep all EXIF
(no prune) for byte-exact MakerNote.

Privacy (partial-strip policies): `MakerNote` is dropped whenever `gps` **or**
`camera` is stripped (it's opaque and can embed GPS/serials); `SubIFDs` (0x014A,
an unmodeled sub-IFD pointer) is dropped on a rewrite rather than left dangling;
IFD1 (thumbnail directory) entries are filtered by the same per-category rules as
IFD0, so a keep-thumbnail policy doesn't leak the Make/Model/DateTime it carries.
The `Web`/`ColorAndRotation` presets drop `gps`/`camera`/`thumbnail`/`other`, so
they were already safe; these close the gaps for hand-rolled `Custom` policies.
Cross-carrier caveat: XMP can duplicate GPS/identity — a policy that keeps XMP
ships it even when the EXIF copy is stripped.

Hardening: bounds-checked, no panics on untrusted input (32M+ fuzz executions);
the serializer dedups aliased out-of-line values to prevent rewrite
memory-amplification; ASCII accessors require the ASCII/UTF-8 TIFF type; thumbnail
length is read as SHORT or LONG; under a stripping policy `retain` fails **safe**
— unparseable or >4 GiB blobs are dropped, never passed through unfiltered.
Validated by differential tests vs `kamadak-exif`, libFuzzer targets, and a
1 KiB–1 MiB-thumbnail zero-copy benchmark.

#### EXIF write / edit path

A blob can be authored from scratch (`Exif::new(TextEncoding)` → setters →
`to_bytes`) or edited after a parse. Setters: `set_copyright` / `set_artist`
insert-or-replace the IFD0 string tag; `set_orientation` insert-or-replaces the
Orientation tag (an existing SHORT/LONG entry keeps its TIFF type, a malformed
non-integer carrier is replaced by the canonical 1-count SHORT, and a tag-less
blob gains one). `to_bytes` re-serializes through the canonical serializer
(offsets recomputed, fixpoint preserved). Mechanism: `Entry.value` is
`Cow<'a, [u8]>`, so parsed entries stay borrowed (zero-copy) while injected
ones are owned.

The caller picks the TIFF type explicitly via `TextEncoding` (`Ascii` = type 2,
`Utf8` = type 129) rather than the writer auto-upgrading to type 129 for
non-ASCII. This is deliberate: type 129 has thin reader support (ExifTool reads
it; kamadak / Pillow / most do not), so an auto-upgrade would silently produce
copyright strings most tools can't read. `Ascii` writes the string's UTF-8 bytes
into the type-2 field (the de-facto interchange form — maximally compatible);
`Utf8` is the spec-conformant choice when the consumer is known to handle it.
Both are NUL-terminated with the count including the NUL.

Still planned (additive, semver-minor; deferred until a concrete consumer):

- Setters for further fields (datetimes, software, …) as consumers appear.
- For broad copyright readability, also writing XMP `dc:rights` (universally
  UTF-8) is the most portable option and a likely companion feature.

Related: the byte-level, offset-preserving rewrite
(`helpers::set_exif_orientation`) remains the *reconciliation* path used by
`Metadata::filtered` to align a baked-upright buffer's embedded tag with the
authoritative `Metadata::orientation` field — it edits an existing tag only and
deliberately never adds one. `Exif::set_orientation` is the *authoring* path.

### `OutputInfo`

Predicted decoder output. Fields: `width`, `height`, `native_format: PixelDescriptor`,
`has_alpha`, `orientation_applied: Orientation`, `crop_applied: Option<[u32; 4]>`.

Methods: `full_decode()`, `buffer_size()`, `pixel_count()`.

### `Cicp`

ITU-T H.273 color description. Re-exported from `zenpixels`. Constants:
`SRGB`, `BT2100_PQ`, `BT2100_HLG`, `LINEAR_SRGB`, `DISPLAY_P3`, `DISPLAY_P3_PQ`.

### `ContentLightLevel` / `MasteringDisplay`

HDR metadata types (CEA-861.3 / SMPTE ST 2086). Re-exported from `zenpixels`.

### `Orientation` / `OrientationHint`

EXIF orientation (1-8 enum) and decode-time orientation strategy
(`Preserve`, `Correct`, `CorrectAndTransform`, `ExactTransform`).

---

## Output types

### `EncodeOutput`

Encoded image bytes. `#[non_exhaustive]`.

Fields: `data: Vec<u8>`, `format: ImageFormat`, `mime_type`, `extension`,
`extensions: Extensions` (type-map, see Extensions section below).

Methods: `new()`, `data()`, `into_vec()`, `format()`, `mime_type()`, `extension()`,
`with_extras<T>()`, `extras<T>()`, `take_extras<T>()`.

Clone drops extras. PartialEq/Eq skip extras.

### `DecodeOutput`

Decoded image with owned pixels. `#[non_exhaustive]`.

Fields: `pixels: PixelBuffer`, `info: ImageInfo`,
`source_encoding: Option<Arc<dyn SourceEncodingDetails>>`,
`extensions: Extensions` (type-map, see Extensions section below).

Methods: `pixels()`, `into_buffer()`, `info()`, `width()`, `height()`,
`has_alpha()`, `descriptor()`, `format()`, `metadata()`,
`with_source_encoding_details<T>()`, `source_encoding_details()`,
`take_source_encoding_details()`,
`with_extras<T>()`, `extras<T>()`, `take_extras<T>()`.

### `AnimationFrame<'a>`

Borrowed animation frame. Fields: `pixels: PixelSlice<'a>`,
`duration: FrameDuration` (exact rational seconds), `frame_index: u32`,
`pts: Option<Timestamp>` (the presentation instant when the source records
one; animation formats that store only durations leave it `None`). Methods:
`duration()`, `pts()`, `with_pts()`, `frame_index()`, `to_owned_frame()`;
`duration_ms()` is deprecated (it truncates).

### `OwnedAnimationFrame`

Owned animation frame. Fields: `pixels: PixelBuffer`,
`duration: FrameDuration`, `frame_index: u32`, `pts: Option<Timestamp>`,
`extensions: Extensions` (type-map).

Methods: `pixels()`, `into_buffer()`, `as_animation_frame()`,
`with_extras<T>()`, `extras<T>()`, `take_extras<T>()`.

### `time`: `TimeBase`, `Timestamp`, `Rounding`

Instants next to durations. `FrameDuration` (in `animation`, re-exported
from `time`) is a nonnegative rational length; `Timestamp { ticks: i64, base:
TimeBase }` is a signed instant on a reduced `TimeBase` (seconds per tick,
`u32 / u32`).

```rust
impl TimeBase {
    const MILLIS: Self; const SECONDS: Self;
    const fn new(numerator: u32, denominator: u32) -> Result<Self, TimingError>;
    const fn hz(ticks_per_second: u32) -> Result<Self, TimingError>;
}
impl Timestamp {
    const fn new(ticks: i64, base: TimeBase) -> Self;
    fn compare(self, other: Self) -> Ordering;                   // exact across bases
    fn same_instant(self, other: Self) -> bool;
    fn rescale(self, target: TimeBase, rounding: Rounding) -> Result<Self, TimingError>;
    fn checked_add(self, duration: FrameDuration) -> Result<Self, TimingError>;
    fn duration_until(self, later: Self) -> Result<FrameDuration, TimingError>;
}
enum Rounding { Floor, Ceil, Nearest }                           // Nearest: ties toward +inf
fn duration_to_ticks(FrameDuration, TimeBase) -> Result<u64, TimingError>;  // exact or Inexact
fn ticks_to_duration(u64, TimeBase) -> Result<FrameDuration, TimingError>;
```

One rounding rule everywhere: `Rounding::Nearest` rounds exact halves toward
+infinity, as `TimingQuantizer` does, so quantizing a run of durations and
rescaling the instants between them give the same endpoints. Missing
timestamps are `Option<Timestamp>`, never sentinels; nothing derives a
timestamp from a nominal frame rate.

### Extensions type-map

`DecodeOutput`, `EncodeOutput`, and `OwnedAnimationFrame` use an `Extensions`
type-map (not a single `Box<dyn Any>`). Multiple independently-typed values
can be stored simultaneously, keyed by `TypeId`. At most one value per
concrete type. Values are `Arc`-wrapped for cheap cloning.

```rust
output.with_extras(gain_map).with_extras(depth_map) // both stored
output.extras::<DecodedGainMap>()  // access gain map
output.extras::<DecodedDepthMap>() // access depth map (independent)
```

### Supplementary decode data conventions

Decode outputs carry three layers of information:

1. **`ImageInfo`** — structured, cross-codec metadata available from probe and
   decode. Always populated. Includes `supplements` flags for discovery.

2. **`SourceEncodingDetails`** — cross-codec encoding analysis (quality estimate,
   lossless detection). Accessed via `source_encoding_details()` and
   `codec_details::<T>()` for the concrete probe struct.

3. **`Extensions` type-map** — supplementary decoded data. Accessed via
   `extras::<T>()`. Multiple types coexist.

#### Discovery, opt-in, then access

Supplement data flows through three stages:

1. **Detection (always, cheap):** `ImageInfo.supplements` and `GainMapPresence`
   are populated during probe/decode from container metadata. No pixel
   decoding occurs. This tells the caller what's available.

2. **Opt-in (caller decides):** Supplement pixel decode is **never automatic**.
   Gain maps and depth maps require explicit opt-in because decoding them
   is expensive (full AV1/HEVC decode at 1/4-1/8 primary resolution).
   Opt-in happens via:
   - Decode node params (e.g., `extract_gain_map: true` on `heic.decode`)
   - Job-level extensions on `DecodeJob`
   - Dedicated codec methods (e.g., `decode_gain_map()`)

3. **Access (typed):** After opt-in decode, supplement pixels appear in
   `extras()`:

```rust
// Stage 1: discover (cheap, always available)
if info.supplements.gain_map {
    // Stage 2: opt-in (caller must request decode separately)
    // Stage 3: access decoded pixels
    let gm = output.extras::<DecodedGainMap>().expect("opted in and decoded");
}
```

**Default behavior:** `Decode::decode()` decodes primary image pixels only.
Supplements are detected but not pixel-decoded unless explicitly requested.
Codecs that currently decode supplements unconditionally should be migrated
to opt-in behavior.

#### Normalized supplement types

Cross-codec supplement types (defined in zencodec) that all codecs should use:

| Type | When to use | Producers |
|------|-------------|-----------|
| `DecodedGainMap` | Decoded gain map pixels + ISO 21496-1 metadata | JPEG (UltraHDR), AVIF (tmap), JXL (jhgm), HEIC (Apple) |
| `DecodedDepthMap` | Decoded depth map pixels | HEIC (Apple), JXL (extra channel) |

When a codec produces a gain map, it MUST use `DecodedGainMap` (not a
codec-specific type) so that consumers work codec-agnostically.

#### Codec-specific extras

Genuinely per-codec data that cannot be normalized belongs in `extras()` with
the codec's own type. Examples:

- JPEG `DecodedExtras` — DCT coefficients, quantization tables, raw APP markers
- HEIC `HeicAuxiliaryInfo` — auxiliary image type list (URNs)
- TIFF `TiffPageInfo` — multi-page IFD metadata

These require `extras::<zenjpeg::DecodedExtras>()` — the caller must know the
codec type. This is intentional: the data is codec-specific by nature.

#### What goes where

| Data | Location | Access |
|------|----------|--------|
| Dimensions, alpha, format, progressive | `ImageInfo` fields | Direct field access |
| ICC, EXIF, XMP, CICP, HDR metadata | `ImageInfo.source_color` / `embedded_metadata` | Direct field access |
| Orientation, resolution, supplements flags | `ImageInfo` fields | Direct field access |
| Quality estimate, lossless detection | `SourceEncodingDetails` trait | `source_encoding_details()` |
| Codec-specific probe data (encoder family, DQT tables, chroma subsampling) | Concrete probe struct | `codec_details::<JpegProbe>()` |
| Gain map pixels + metadata | `DecodedGainMap` in extensions | `extras::<DecodedGainMap>()` |
| Depth map pixels | `DecodedDepthMap` in extensions | `extras::<DecodedDepthMap>()` |
| Codec-specific decode artifacts | Codec's own type in extensions | `extras::<CodecSpecificType>()` |

---

## Format detection

### `ImageFormat`

```rust
enum ImageFormat {
    Jpeg, Png, Gif, WebP, Avif, Jxl, Heic, Bmp, Tiff, Ico, Pnm, Farbfeld, Qoi, Unknown,
    Custom(&'static ImageFormatDefinition),
}
```

Methods: `from_magic(data)`, `definition()`, `mime_type()`, `extension()`,
`display_name()`, `supports_alpha()`, `supports_animation()`, etc.

### `ImageFormatDefinition`

Metadata for a format: name, extensions, MIME types, capability flags, detection function.

### `ImageFormatRegistry`

Thread-safe registry for custom formats. `common()` returns built-in formats.

---

## CodecSet (multi-codec registry)

Runtime set of registered codec configs with one entry point per operation.
`Send + Sync + 'static`, `Clone`, `Debug`, `Default`; every operation takes
`&self`, so one instance can be shared app-wide (`LazyLock` / `OnceLock` /
`Arc`, or `Box::leak` in `no_std`).

```rust
struct CodecSet { /* private */ }

impl CodecSet {
    fn with_collection(self, factory: impl collection::CollectionFactory + 'static) -> Self;
    fn open_collection<'a, R: operation::OperationPolicy>(&self, format: ImageFormat,
        source: &'a mut dyn source::DynReadAt, request: collection::OpenRequest,
        resolver: R, pulse: SharedPulse)
        -> Result<operation::CheckedCollection<collection::Discovery<collection::ErasedCollection<'a>>, R>,
            At<operation::CollectionOperationError>>;
    fn new() -> Self;

    // Registration — self-describing via DecoderConfig::formats() /
    // EncoderConfig::format(); first registered wins per format.
    fn with_decoder(self, config: impl DecoderConfig + 'static) -> Self;
    fn with_encoder<C>(self, config: C) -> Self
        where C: EncoderConfig + 'static,
              <C::Job as EncodeJob>::Enc: Encoder + Send,
              <C::Job as EncodeJob>::AnimationFrameEnc: AnimationFrameEncoder;

    // Defaults stamped onto every job the set creates.
    fn with_limits(self, limits: ResourceLimits) -> Self;
    fn with_pulse(self, pulse: SharedPulse) -> Self;
    fn with_decode_policy(self, policy: DecodePolicy) -> Self;
    fn with_encode_policy(self, policy: EncodePolicy) -> Self;

    // Queries.
    fn detect(&self, data: &[u8]) -> Option<ImageFormat>;
    fn can_decode(&self, format: ImageFormat) -> bool;
    fn can_encode(&self, format: ImageFormat) -> bool;
    fn decoder_for(&self, format: ImageFormat) -> Option<&dyn DynDecoderConfig>;
    fn encoder_for(&self, format: ImageFormat) -> Option<&dyn DynEncoderConfig>;
    fn decodable_formats(&self) -> impl Iterator<Item = ImageFormat> + '_;
    fn encodable_formats(&self) -> impl Iterator<Item = ImageFormat> + '_;

    // Decode: detect → stamped job → run.
    fn probe<'a>(&'a self, data: &'a [u8]) -> Result<ImageInfo, CodecSetError>;
    fn decode<'a>(&'a self, data: &'a [u8]) -> Result<DecodeOutput, CodecSetError>;
    fn decode_preferring<'a>(&'a self, data: &'a [u8], preferred: &[PixelDescriptor])
        -> Result<DecodeOutput, CodecSetError>;
    fn decode_as<'a>(&'a self, format: ImageFormat, data: &'a [u8], preferred: &[PixelDescriptor])
        -> Result<DecodeOutput, CodecSetError>;
    fn push_decode<'a>(&'a self, data: &'a [u8], sink: &mut dyn DecodeRowSink,
        preferred: &[PixelDescriptor]) -> Result<OutputInfo, CodecSetError>;
    fn animation_decoder<'a>(&'a self, data: &'a [u8], preferred: &[PixelDescriptor])
        -> Result<Box<dyn DynAnimationFrameDecoder>, CodecSetError>;   // 'static result
    fn streaming_decoder<'a>(&'a self, data: &'a [u8], preferred: &[PixelDescriptor])
        -> Result<Box<dyn DynStreamingDecoder + 'a>, CodecSetError>;   // borrows set + data
    fn decode_job<'a>(&'a self, format: ImageFormat)
        -> Result<Box<dyn DynDecodeJob<'a> + 'a>, CodecSetError>;      // escape hatch (hints, etc.)

    // Encode: format-keyed; the registered config is a template.
    fn encode(&self, format: ImageFormat, pixels: PixelSlice<'_>)
        -> Result<EncodeOutput, CodecSetError>;
    fn encode_with(&self, format: ImageFormat, fidelity: Fidelity, pixels: PixelSlice<'_>)
        -> Result<EncodeOutput, CodecSetError>;                        // clones the template
    fn encode_job(&self, format: ImageFormat) -> Result<Box<dyn DynEncodeJob>, CodecSetError>;
    fn encode_job_with(&self, format: ImageFormat, fidelity: Fidelity)
        -> Result<Box<dyn DynEncodeJob>, CodecSetError>;
    // One-call proxy op: decode -> carry source metadata (filtered) -> encode.
    // Single-image, no pixel processing, no descriptor adaptation.
    fn transcode(&self, input: &[u8], target: ImageFormat, fidelity: Fidelity,
        metadata: MetadataPolicy, color: ColorEmitPolicy)
        -> Result<EncodeOutput, CodecSetError>;

    // Resource estimation: forward to the registered codec's cost model
    // (unknown() if it has none); NoEncoder / NoDecoder if unregistered.
    fn estimate_encode(&self, format: ImageFormat, image: &ImageCharacteristics,
        compute: &ComputeEnvironment) -> Result<ResourceEstimate, CodecSetError>;
    fn estimate_decode(&self, format: ImageFormat, image: &ImageCharacteristics,
        compute: &ComputeEnvironment) -> Result<ResourceEstimate, CodecSetError>;
    // Pixels-based: reads dims + descriptor off the slice, then estimate_encode.
    fn estimate_encode_of(&self, format: ImageFormat, pixels: PixelSlice<'_>,
        compute: &ComputeEnvironment) -> Result<ResourceEstimate, CodecSetError>;
    // Bytes-based: probe → estimate_decode at the decoder's native output.
    fn estimate_decode_of(&self, data: &[u8], compute: &ComputeEnvironment)
        -> Result<ResourceEstimate, CodecSetError>;
}

enum CodecSetError {
    UnrecognizedFormat,        // detect() matched nothing registered
    NoDecoder(ImageFormat),    // known format, nothing registered for it
    NoEncoder(ImageFormat),
    Codec(BoxedError),         // codec failure; source() exposes the chain
}
```

Semantics:

- **Detection** consults only formats with a registered decoder: built-ins in
  `ImageFormatRegistry::common()` priority order (so AVIF-before-HEIC and
  DNG-before-TIFF disambiguation is preserved regardless of registration
  order), then `Custom` formats in registration order.
- **Encoder templates**: codec-specific options are set on the concrete config
  before registration; `encode_with` / `encode_job_with` clone the template
  and apply a per-call `Fidelity` to the clone.
- **Job escape hatches**: `decode_job` / `encode_job` return the stamped
  `Dyn*Job` for per-operation control (decode hints, metadata, canvas/loop
  settings) before running an executor.

## Prelude

`zencodec::prelude::*` imports every encode/decode trait (generic and dyn
variants) so `.job()`, `.decoder()`, `.encode()`, `.next_batch()`, … resolve
with one `use`. Types are not included.

---

## Capabilities

### `EncodeCapabilities` / `DecodeCapabilities`

Const-constructible structs with builder pattern. Returned by config `capabilities()`.

**`EncodeCapabilities` accessors:** `icc`, `exif`, `xmp`, `cicp`,
`cicp_is_valid_carrier`, `cicp_safe_sole_carrier`, `stop`, `animation`,
`animation_timing`, `push_rows`, `streaming_output`, `encode_from`, `lossy`,
`lossless`, `hdr`, `gain_map`, `native_gray`, `native_16bit`, `native_f32`,
`native_alpha`, `enforces_max_pixels`, `enforces_max_memory`, `effort_range`,
`quality_range`, `threads_supported_range`.

**`DecodeCapabilities` accessors:** `icc`, `exif`, `xmp`, `cicp`, `stop`,
`animation`, `multi_image`, `cheap_probe`, `decode_into`, `streaming`,
`incremental`, `seekable`, `access_pattern`, `hdr`, `gain_map`, `inventory`,
`reconstructs_hdr`, `native_gray`, `native_16bit`, `native_f32`,
`native_alpha`, `enforces_max_pixels`, `enforces_max_memory`,
`enforces_max_input_bytes`, `threads_supported_range`.

Each has a `with_*` builder.

Sans-I/O flags: `streaming_output` (encode: some configuration streams
bytes from `pending_output` after `begin`; the `OutputTiming` that `begin`
returns says whether a given encoder does); `incremental`, `seekable` and
`access_pattern` (decode: `dyn_incremental_decoder`,
`push_decoder_from_source`, and how the incremental decoder reads).

Method: `supports(UnsupportedOperation) -> bool`.

### `UnsupportedOperation`

```rust
enum UnsupportedOperation {
    RowLevelEncode, PullEncode, AnimationEncode,
    DecodeInto, RowLevelDecode, AnimationDecode,
    PixelFormat,
}
```

---

## Resource limits

### `ResourceLimits` (`Copy + Clone + Debug + PartialEq + Eq`)

Fields: `max_pixels`, `max_memory_bytes`, `max_output_bytes`, `max_width`,
`max_height`, `max_input_bytes`, `max_frames`, `max_animation_ms`,
`threading: ThreadingPolicy`.

Validation methods: `check_dimensions()`, `check_memory()`, `check_image_info()`,
`check_output_info()`, `check_decode_cost()`, `check_encode_cost()`.

### `LimitExceeded`

Error enum: `Width`, `Height`, `Pixels`, `Memory`, `InputSize`, `OutputSize`,
`Frames`, `Duration` — each carries `actual` and `max`.

### `ThreadingPolicy`

```rust
enum ThreadingPolicy {
    SingleThread,
    LimitOrSingle { max_threads: u16 },
    LimitOrAny { preferred_max_threads: u16 },
    Balanced,
    Unlimited,  // #[default]
}
```

---

## Security policies

### `DecodePolicy` / `EncodePolicy`

Const-constructible structs controlling what metadata to extract/embed,
what features to allow.

**`DecodePolicy` flags:** `allow_icc`, `allow_exif`, `allow_xmp`, `allow_progressive`,
`allow_animation`, `allow_truncated`, `strict`.

**`EncodePolicy` fields:** `color: Option<ColorEmitPolicy>` (ICC-vs-CICP carrier,
read by the codec via `resolve_color`) plus the coarse, best-effort per-channel
embed gates `embed_icc`, `embed_exif`, `embed_xmp`. The embed gates are *not* the
reliable retention control — for field-level privacy use
`EncodeJob::with_metadata_policy`.

`DecodePolicy` constructors: `none()`, `strict()`, `permissive()`.
`EncodePolicy` constructors: `none()`, `strip_all()`, `preserve_all()`;
builder `with_color()`.

---

## Color types

`Cicp`, `ContentLightLevel`, and `MasteringDisplay` are re-exported from `zenpixels`.
See `zenpixels` documentation for field details.

---

## Color emission

`resolve_color_emit(&SourceColor, &EncodeCapabilities, ColorEmitPolicy) -> ColorEmitPlan`
— a pure, `no_std`, CMS-free decision of which color carriers an *encode* writes for
a target. Crate-root re-exports (`CicpEmission`, `ColorEmitFields`, `ColorEmitPlan`,
`ColorEmitPolicy`, `IccDisposition`, `resolve_color_emit`).

```rust
pub fn resolve_color_emit(
    source: &SourceColor,
    caps: &EncodeCapabilities,
    policy: ColorEmitPolicy,
) -> ColorEmitPlan;

#[non_exhaustive]
pub enum ColorEmitPolicy { Compatibility, Balanced, Compact, Verbatim, Custom(ColorEmitFields) }

pub struct ColorEmitPlan { pub cicp: Option<Cicp>, pub icc: IccDisposition }

#[non_exhaustive]
pub enum IccDisposition { KeepSource, SynthesizeFrom(Cicp), Drop }
```

- **`ColorEmitPolicy`** — `Balanced` (default) writes CICP where it's a *safe sole
  carrier* and keeps a synthesized ICC companion otherwise (e.g. PNG `cICP`);
  `Compatibility` favors the widest reader support; `Compact` prefers CICP and drops
  the ICC; `Verbatim` carries the source's signals unchanged; `Custom(ColorEmitFields)`
  is explicit (`ColorEmitFields::new`).
- **`EncodeCapabilities`** carrier methods: `cicp_is_valid_carrier` (the format has a
  standardized CICP slot — JXL/AVIF/HEIC `nclx`, PNG `cICP`) and `cicp_safe_sole_carrier`
  (CICP alone reaches every measured reader — JXL only; AVIF/HEIC and PNG keep
  an ICC alongside, imazen/zencodec#132).
- **`IccDisposition`** — `KeepSource` re-embeds the source ICC; `Drop` emits none;
  `SynthesizeFrom(cicp)` asks the caller to materialize bytes (this crate carries no
  CMS) via `zenpixels_convert`'s transfer-aware `synthesize_icc_for_cicp` — a bundled
  `const` profile or a CMS-generated one, never a mis-tagged TRC. The plan never emits
  a redundant `SynthesizeFrom(sRGB)`.

The emit-direction names can't be confused with the decode-side `SourceColor`. Design
and rejected alternatives: `docs/color-emit-model.md`; how the framework resolves
color/orientation/metadata before a codec runs: `docs/correctness-model.md`.

---

## Error utilities

### `ErrorCategory` (enum) + `CategorizedError` (trait)

Coarse, codec-agnostic error classification for routing (HTTP status, retry,
logging) without naming the concrete error enum.

```rust
#[non_exhaustive]
enum ErrorCategory {
    Image(ImageError),        // the bytes are the problem
    Request(RequestError),    // the caller's request is the problem
    Resource(ResourceError),  // a cap was hit, or allocation failed
    Policy(PolicyKind),        // valid input a configured policy declined
    Stopped(StopReason),    // stopped via the Stop token (Cancelled / TimedOut)
    Io(CodecIoKind),          // an I/O or output-sink failure
    Internal(InternalKind),   // a bug / broken invariant / unclassified dependency error
}

#[non_exhaustive] enum ImageError { Malformed, UnexpectedEof, Unsupported(UnsupportedImageKind) }
#[non_exhaustive] enum UnsupportedImageKind { Type, Feature }
#[non_exhaustive] enum RequestError { Invalid(InvalidKind), Unsupported(UnsupportedOperation), CmsRequired }
#[non_exhaustive] enum InvalidKind { Parameters, Buffer, State }
#[non_exhaustive] enum ResourceError { Limits(LimitKind), OutOfMemory }
#[non_exhaustive] enum PolicyKind { Decode, Encode }      // mirrors DecodePolicy / EncodePolicy
#[non_exhaustive] enum InternalKind { Bug, Dependency }   // Bug = our defect; Dependency = unclassified foreign error
// StopReason is enough::StopReason { Cancelled, TimedOut } — reused, not re-defined.

trait CategorizedError: core::any::Any {
    fn codec_name(&self) -> Option<&'static str>;  // required; cause types return None
    fn category(&self) -> ErrorCategory;
}
```

**Origin-first shape.** The top level splits by *who owns the fault*, so a generic
consumer can route on the outer arm and only destructure when a sub-kind changes
the answer. `Image(_)` is "the bytes are the problem" (a *different* codec might
handle them; the caller can't fix it by changing parameters) — this whole arm is
the client-supplied-data / incomplete-input set a truncation check tolerates.
`Request(_)` is "the *request* is the problem" (the caller can change config,
buffer, call sequence, or the operation/format asked for).

The "unsupported" axis is split by origin: `Image(Unsupported(Type))` (the format
isn't handled at all), `Image(Unsupported(Feature))` (a bitstream feature within a
handled format), and — on the request side — `Request(Unsupported(op))` (an API
operation this codec doesn't do, including its `PixelFormat` arm when negotiation
found no common `PixelDescriptor`). `Request(CmsRequired)` flags a
colour-management transform the codec won't perform itself.
`Policy(kind)` is valid input the codec *could* handle but a configured policy
declined — `PolicyKind` mirrors the crate's existing `DecodePolicy` / `EncodePolicy`
split (e.g. progressive content rejected by a decode policy, or alpha removal
forbidden by an encode policy), so the call site already knows which one.
`Internal(kind)` splits similarly for telemetry/triage, not routing: `Bug` is a
broken invariant in the codec's own logic (never retryable, always alert-worthy);
`Dependency` is an error surfaced from a sub-component/foreign library the codec
hasn't classified into `Image`/`Request`/`Resource` — an honest "unclassified",
not a permanent home. Both `Internal` variants still mean "500" for routing; the
split only pays off when telemetry carries the category forward without a
per-codec downcast.
`Request(Invalid(Buffer))` is a wrong-geometry pixel buffer
(size/stride/alignment/descriptor) and `Request(Invalid(State))` is API misuse
(called out of sequence) — both distinct from `Request(Invalid(Parameters))`
(config/knobs). `Resource(Limits(kind))` is a configured cap; `Resource(OutOfMemory)`
is genuine allocation exhaustion. `Io` carries a `CodecIoKind` — a
`std::io::ErrorKind` when the `std` feature is enabled, empty under `no_std` (the
variant shape is stable either way, so matching `Io(_)` is portable), anticipating
a future `core::io::ErrorKind`.
The set is distilled from a per-codec inventory; see
[`error-taxonomy-inventory.md`](error-taxonomy-inventory.md) for the
variant→category mapping and right-sizing rationale, and
[`error-types-ecosystem.md`](error-types-ecosystem.md) for the wider ecosystem.

Sub-enums carry `From` shortcuts into `ErrorCategory` (`ImageError`, `RequestError`,
`ResourceError`, `StopReason`, and the leaf kinds `UnsupportedImageKind` /
`InvalidKind` / `LimitKind`), so a codec's `category()` arm reads
`ImageError::Malformed.into()` or `InvalidKind::Buffer.into()` rather than spelling
the outer wrapper each time.

`CategorizedError` is **opt-in** (not blanket-implemented): a codec implements it
on its error type, mapping each variant to one category. It is **not** required
by the `EncoderConfig`/`DecoderConfig::Error` bound, so adopting it is additive
and back-compatible. A blanket `impl<E: CategorizedError> CategorizedError for
whereat::At<E>` forwards to the inner error, so a located error keeps its
category. zencodec's own cause types implement it (`LimitExceeded` →
`Resource(Limits(kind))`, `UnsupportedOperation` → `Request(Unsupported(self))`,
`enough::StopReason` → `Stopped(self)` — the reason IS the payload, no lossy
collapse), so a codec's mapping is usually a one-line delegation per arm.
`LimitKind` is the value-free discriminant of `LimitExceeded`
(`LimitExceeded::kind()`).

The `codec_name()` method is where a codec declares its name —
`fn codec_name(&self) -> Option<&'static str> { Some("zenjpeg") }` — so
[`CodecError::from_native`] / [`of`] tag the envelope from the value instead of
taking a codec argument. It is **required** (no default), so every implementor
answers it; the cause types return `None` (they aren't codecs); `At<E>` forwards
the inner `codec_name()`. It is a `&self` method rather than an associated const
specifically so the trait stays **dyn-compatible**: with the `Any` supertrait a
`dyn CategorizedError` can be formed *and* downcast to its concrete type — an
associated const would forbid the trait object outright.

### `CodecError` (struct)

The shared error envelope: a coarse `ErrorCategory`, the originating codec's name,
and (optionally) the codec's own detail error. A codec returns it as
`whereat::At<CodecError>` — the recommended (and only needed) form, with the
cleanest `?` / `.at()` ergonomics.

`CodecError` is a **one-word handle** — its fields live behind a `Box` — so
`At<CodecError>` is **two words** (handle + trace, 16 bytes on 64-bit) and every
`Result<_, At<CodecError>>` a codec threads through `?` is two words too (the box
pointer's niche absorbs the `Result` discriminant). That is small enough to return
in registers rather than spill to the stack — the reason for the box: the detail is
a *fat* `Box<dyn Error>` (16 bytes alone), so an unboxed envelope would exceed the
16-byte ABI threshold and spill regardless of how thin the other fields are. The
trade is one cold-path allocation per error, fine for an error type; `new` is
therefore no longer `const`. Recovery (`codec_error()` / `error_category()`) is
downcast-based and additionally tolerates a single consumer-applied `Box` layer in
either position (`Box<At<CodecError>>`, `At<Box<CodecError>>`, `Box<CodecError>`);
deeper nesting isn't covered.

The codec name is an `Option<&'static str>` (`None` when unset — honest, no
sentinel). `from_native` / `of` read it from the detail's
[`codec_name()`](#errorcategory-enum--categorizederror-trait); `new` / `from_parts`
(no typed detail) take it directly. `codec()` reads it back.

```rust
struct CodecError(Box<Repr>);  // Repr { category, codec: Option<&'static str>, detail: Option<Box<dyn Error + Send + Sync>> }

impl CodecError {
    fn new(codec: Option<&'static str>, category: ErrorCategory) -> Self;                     // no detail
    fn from_native<E: CategorizedError + Error + Send + Sync + 'static>(detail: E) -> Self;    // bare; name from detail.codec_name()
    fn of<E: CategorizedError + Error + Send + Sync + 'static>(located: At<E>) -> At<CodecError>;  // located; trace preserved
    fn from_parts(codec: Option<&'static str>, category: ErrorCategory, detail: Box<dyn Error + Send + Sync>) -> Self;
    fn with_codec(self, codec: Option<&'static str>) -> Self;  // builder: stamp/clear the codec name on an existing envelope
    fn category(&self) -> ErrorCategory;             // total, fixed at construction
    fn codec(&self) -> Option<&'static str>;         // which codec produced it (None if unset)
    fn detail(&self) -> Option<&(dyn Error + 'static)>;
}
```

Two ways to surface a codec error, both routable by category:

- **Native enum + `CategorizedError`** (`type Error = MyError`): `category()`
  classifies on the *typed* path. Once erased to `Box<dyn Error>` the category is
  unreachable — the erased value is a `dyn Error`, not a `dyn CategorizedError`.
- **The envelope** (`type Error = whereat::At<CodecError>`): because
  `At<CodecError>` is one *concrete* type, the category **and** the codec name
  survive erasure. A consumer recovers them from any `Box<dyn Error>` /
  `anyhow::Error` / mapped wrapper by downcast — see `CodecErrorExt::codec_error()`
  / `error_category()`.

The **`codec` name** (`&'static str`, e.g. `"zenjpeg"`) is how a consumer tells
codecs apart without naming any codec-specific type — useful when several codecs
feed one pipeline. The **`detail` is optional**: `CodecError::new(codec, category)`
is a complete error for a codec that has no error enum of its own.

`from_native` / `of` read the category *and* the codec name from the detail's
`CategorizedError` impl at construction, so both are total and never re-derived
from an opaque chain. `of` takes an **already-located** `At<E>` (not a bare `E`):
location is mandatory at the type level — a codec that skipped whereat can't call
it — and the trace stays on the *outside* (`At<CodecError>`, mapping the inner
error via whereat's trace-preserving `map_error`), never buried in the detail.
`Display` is `"{codec}: {detail-or-category}"`; `source()` is the detail (when
present), so the typed extractors below still reach the underlying cause.

**Adoption is one impl.** A codec keeping a native `MyError: CategorizedError`
adds `impl From<MyError> for At<CodecError>` (body: `CodecError::of(e.start_at())`,
with `fn codec_name(&self) { Some("mycodec") }` on the error's `CategorizedError`
impl), after which `?` on any `Result<_, MyError>` auto-wraps into the envelope —
existing fallible internals need no rewrite. (Or convert a bare `Result<_, MyError>`
in one step with whereat's `map_err_at(CodecError::from_native)`.) Reject stubs use `Unsupported<At<CodecError>>`. `At` and the
`start_at()` / `.at()` ext traits are **re-exported** from the crate root
(`zencodec::{At, ErrorAtExt, ResultAtExt}`), so a codec can name
`zencodec::At<zencodec::CodecError>` without depending on `whereat` directly. The
testkit's `minimal` codec is a worked example; `reference` keeps the native-enum
form for contrast.

#### Attaching structural locus (`StreamOffset`)

Per-variant detail (dimensions, expected/actual, table indices) stays in the
native error — reachable via `detail` / `find_cause`. The one piece of context
worth carrying *generically* is **where in the input** the failure was: best-in-
class decoders report a byte offset, and zen decoders already track it internally.
Rather than grow a per-locus field on `CodecError`, a codec attaches it to the
`At<CodecError>` trace as typed context, and a generic consumer recovers it by
downcast — no codec-specific type named:

```rust
// codec, at the failure site (offset = bytes consumed):
Err(at!(CodecError::new(Some("zenjpeg"), ErrorCategory::Image(ImageError::Malformed)))
    .at_data(|| StreamOffset(reader.position())))

// consumer, even after erasure to Box<dyn Error>:
let off = err.contexts().find_map(|c| c.downcast_ref::<StreamOffset>().copied());
```

`StreamOffset(pub u64)` is a shared, downcast-able newtype (bytes from the start
of the encoded input) so the "where" convention is cross-codec. Richer loci
(marker, box fourcc, NAL type, frame index) ride the same `at_data` / `at_str`
channel; only the offset is standardized as a type today.

### `CodecErrorExt` (trait)

Extension trait for inspecting error chains without downcasting — the *detail*
layer beneath `ErrorCategory` (which cap, which operation, which cause), plus
`codec_error()` / `error_category()` which recover the `CodecError` envelope (and
hence the category + codec name) from any erased error (`Box<dyn Error>`,
`anyhow`):

```rust
trait CodecErrorExt {
    fn unsupported_operation(&self) -> Option<&UnsupportedOperation>;
    fn limit_exceeded(&self) -> Option<&LimitExceeded>;
    fn codec_error(&self) -> Option<&CodecError>;       // recover the envelope post-erasure
    fn error_category(&self) -> Option<ErrorCategory>;  // = codec_error().map(|c| c.category())
    fn find_cause<T: core::error::Error + 'static>(&self) -> Option<&T>;
}
```

### `find_cause<T>(err) -> Option<&T>`

Walk an error chain looking for a specific cause type.

### `Unsupported<E>`

Generic stub type for unsupported decode modes. Implements `StreamingDecode`
and `AnimationFrameDecoder` with unreachable bodies. Use as `type StreamDec = Unsupported<E>`.

---

## Source encoding detection

### `SourceEncodingDetails` (trait)

Codec-agnostic interface for querying how an image was encoded. Each codec's
probe type (e.g. `JpegProbe`, `WebPProbe`) implements this trait.

```rust
trait SourceEncodingDetails: Any + Send + Sync {
    fn source_generic_quality(&self) -> Option<f32>;
    fn is_lossless(&self) -> bool;  // default: false
}

impl dyn SourceEncodingDetails {
    fn codec_details<T: SourceEncodingDetails + 'static>(&self) -> Option<&T>;
}
```

`source_generic_quality()` returns a 0.0–100.0 estimate on the same scale as
`EncoderConfig::with_generic_quality()`. Returns `None` for lossless encodings
or when quality can't be determined from headers. Approximate (±5).

The trait intentionally has very few methods — only properties meaningful across
all image formats. Codec-specific details (color type, bit depth, palette size,
chroma subsampling, encoder family, quantizer tables) belong on the concrete
probe struct and are accessed via `codec_details::<T>()` downcast.

Available on both `ImageInfo` (from probe or decode) and `DecodeOutput`. Codec
implementors populate it when the codec can detect source encoding properties
from headers.

---

## Re-exports

```rust
pub use enough;             // cooperative cancellation (Stop trait)
pub use enough::Unstoppable;
pub use almost_enough::StopToken;  // cancellation-only compatibility
pub use how_far;
pub use how_far::{NoPulse, Pulse, SharedPulse};  // operation control
```

---

## Helpers

### `copy_decode_to_sink()`

Fallback `push_decoder` implementation via one-shot decode + copy to sink.
For codecs that can't stream natively.

### `copy_frame_to_sink()`

Fallback `render_next_frame_to_sink` implementation via `render_next_frame` + copy.

### `negotiate_pixel_format()` / `best_encode_format()` / `is_format_available()`

Format negotiation helpers for matching preferred descriptors to codec capabilities.

### Sample-exact output planning

The conversion dependency exposes `OutputPlan::new_preserving_samples(source,
current, origin, target_profile, required_descriptor)`. Matching current/target
profiles and a reversible representation proof are required. Identity, supported
channel reorder/expansion and U8-to-U16 widening preserve represented samples;
color arithmetic, reassociation, depth reduction and channel removal refuse
before execution. No CMS is consulted. This is distinct from unclipped color
conversion, whose arithmetic is not bit-exact. The native JXL example in
`perf/spatial-validation/src/jxl_planned_color_tests.rs` exercises borrowing and
opaque-alpha addition against actual decoded samples.

The native conversion qualification also covers required unclipped Gamma22
execution. The previous route accepted the option but clipped actual negative
AV1-derived samples. Internal extended steps now preserve that range while
retaining positive in-range and default-clipped results. Raw native traces and
an independent reader qualify the existing Adobe-exponent model. This does not
establish universal numerical, HDR or CMS guarantees; see the native AV1 README.

## Synchronous container output (`zencodec-media/std`)

The sans-I/O `Muxer` remains unchanged. `zencodec_media::output` adds checked
synchronous writer ownership as an opt-in adapter:

```rust
impl<M: Muxer, D: Destination> PreparedOutput<M, D> {
    pub fn new(mux: M, destination: D, tracks: &[TrackInfo], pulse: &dyn Pulse)
        -> Result<Self, Failure>;
    pub const fn timing(&self) -> MuxTiming;
    pub fn publish(self, pulse: &dyn Pulse) -> Result<Output<M, D>, Failure>;
}
impl<M: Muxer, D: Destination> Output<M, D> {
    pub fn new(mux: M, destination: D, tracks: &[TrackInfo], pulse: &dyn Pulse)
        -> Result<Self, Failure>;
    pub fn with_container_timing(mux: M, destination: D, tracks: &[TrackInfo], pulse: &dyn Pulse)
        -> Result<Self, Failure>;
    pub const fn stats(&self) -> &Stats;
    pub fn is_failed(&self) -> bool;
    pub fn send(&mut self, packet: Packet, pulse: &dyn Pulse) -> Result<(), Failure>;
    pub fn finish(self, pulse: &dyn Pulse) -> Result<Stats, Failure>;
}
```

Both `new` constructors require `begin_exact_timing`; only
`with_container_timing` opts into ordinary native timing. Prepared output owns
its muxer and destination but writes nothing until consumed by `publish`.
`Forward<W: Write>` and `SeekBack<W: Write + Seek>` implement the sealed
`Destination` interface. Seek-back offsets are relative to the initial position;
finish restores the final position after applying validated patches.

`Failure` exposes its `Stage`, confirmed appended `emitted` bytes and owned
original `cause`. Stages are `Prepare`, `Packet`, `Drain`, `Finish` and `Patch`.
`Stats` reports emitted bytes, completed patches, patch bytes and maximum pending
slice length. These are progress observations, not memory bounds. A failed
patch may leave partial overwrite effects. Send errors and unwind make the
operation terminal; dropping never publishes or finishes implicitly.

Native validation uses this public driver directly. The complete frame-route
facade and destination evidence in the shared policy record remain open. These
blocking writers do not establish safe backpressure against a live encoder.

## Rendering companion

`zencodec_render::Renderer::new(storage)` configures full oriented sRGB RGBA8
rendering. `with_request(RenderRequest)` and `with_cms(&dyn PluggableCms)`
configure explicit output and color processing. `RenderRequest::new(descriptor)`
requires current-space output with channel/depth restrictions; `with_target`,
`with_region` and `with_conversion` change those requirements explicitly.

`render_first(&mut CheckedCollection<C, R>)` returns
`Result<Option<Outcome<ImageOutput>>, At<CollectionOperationError>>`: the first
top-level image, or `None` only on successful empty discovery. It propagates
errors without skipping selections and restarts discovery on each call.

`render_selected(&mut CheckedCollection<C, R>, &ImageId)` returns
`Result<Outcome<ImageOutput>, At<CollectionOperationError>>` and uses the stored
operation policy once, after rendering. `render(ImageOutput, &mut policy,
pulse)` returns `Result<Outcome<ImageOutput>, At<RenderFailure>>` for an
already-held image. The policy implements `OperationPolicy`; the control is
`&dyn Pulse`. The deprecated compatibility call is
`render_with_preset(ImageOutput, &mut resolver, preset, pulse)`. `RenderFailure` exposes `RenderStage` (ColorPlan, Geometry,
Color, Publication) and retains the original error as its source.

The companion and `geometry::project` perform no source I/O. See the
[rendering contract](render-contract.md) for defaults, refusals, ownership and
remaining limits. This review crate is separate from the root workspace until
its required converter revision is published.

### Destination evidence at frame-route policy

`FrameRouteOffer::destination: Option<DestinationFacts>` describes the actual
retained writer/container: `seek_back`, `streams`, `patches_at_finish`, and
`exact_timing`. `None` means unqualified. `PreparedOutput::facts()` and
`Output::facts()` derive these values from their actual retained objects and
successful preparation. Ordinary `with_container_timing` does not claim exact
timing, even when a particular file happens to preserve it.

The selected offer is retained in the bounded decision report. These are facts,
not capability coordination required of callers: unsupported layouts still fail
at preparation. Adapters must retain the objects described by their offer. Byte
streaming says nothing about encoder raw-buffer ownership and never establishes
safe backpressure or a whole-operation memory bound.

## Owned finite video encoding

`zencodec_media::staged::StagedEncode::new(encoder, muxer, destination, resolver,
storage, pulse)` prepares exact-timing single-track output from the retained
encoder configuration. `push(Frame, pulse)` retains the exact picture through
policy. `push_converted(Frame, &FrameInterpretation, FrameConversionOffer,
FnOnce(Frame) -> Result<Frame, Cause>, pulse)` retains a prepared conversion,
executes it once after selection and checks full output interpretation/timing.

`finish(pulse)` consumes the operation, drains and drops its encoder, then writes
the retained destination. It returns `Completed { stats, report }`. Failure or
unwind is terminal; `report()` remains readable after a failed push. Located
`Failure` retains its `Stage`, decision report and concrete cause. `Refusal`
provides typed staging/protocol failures. Underlying output failures retain
confirmed partial effects. Dropping performs no implicit finish or publication.

By default this std convenience stages the full compressed sequence in managed
payload memory, including packet additions. It preserves packet timing and side data.
Bookkeeping, native workspace and caller-owned pictures are outside that budget.
It requires output configuration before input and a fixed configuration; it does
not segment, infer fidelity or provide live encoder backpressure.
The underlying encoder and muxer mechanisms remain sans-I/O.

`VideoEncoder::output_fidelity() -> Option<zencodec::encode::Fidelity>` describes
the guarantee of the retained configured encoder for all accepted inputs.
Unknown is the default. All shared encoder wrappers forward the method.
`StagedEncode` binds it at preparation, supplies it to frame-route policy, and
rejects drift during push/drain before publication. This is automatic adapter
coordination, not a query callers must perform. Preceding conversion fidelity
remains a separate offer field.

Explicit disk staging uses `StagedEncode::with_staging(..., Staging::disk(file,
DiskLimits::new(bytes, packets, additions_per_packet), replay_budget)?, pulse)`.
The caller supplies an exclusively controlled scratch file positioned at its end;
existing prefixes are preserved. File privacy, retention and deletion belong to
the caller. No automatic disk fallback occurs. Policy sees `StagingFacts`
(kind, accumulated payload bytes, payload limit) before accepting each frame.
Disk writes can block while the encoder retains raw state; only the final
destination waits until the encoder drops.

Disk payload, packet count and per-packet addition count have separate caps.
Replay admits owned payloads after encoder release; muxer retention keeps those
charges, so later admission can fail. The first packet is read before publishing
the header. Later read/admission failures may follow destination effects:
`staged::Failure::emitted` retains confirmed appends and the original cause.
Patch failures may also leave partial overwrites. These are not transactions
or whole-operation memory guarantees.

### Prepared output scheduling

`Question::OutputSchedule` carries `OutputScheduleFacts { timing, source }`
and retained `OutputScheduleOffer` alternatives. `OutputSchedule::FinishFirst`
releases the encoder before final-destination I/O; `Live` permits waits while
the encoder is alive. `OutputSource::{BorrowedRaster, PullRows, Unknown}`
describes acquisition, not a memory guarantee. A pull source can itself own a
full raster. Neither schedule implies release of caller-owned inputs.

`resolve_output_schedule` uses the existing fallible resolver, scoped tokens
and bounded records. A live offer contradicting `AT_FINISH` refuses before
selection. Adapters keep prepared objects across selection, perform no automatic
fallback, and retain the decision record on subsequent execution errors.

`PolicySet::require_output_schedule` selects only the requested mechanism.
`with_output_wait_limit` requires a qualified bound on all route-retained
memory during destination waits, including source, codec, conversion, metadata,
scratch and compressed data. Unknown refuses even for finish-first. It excludes
unrelated application state and destination-owned allocations and is not a peak
operation bound. Pass the requirements through `PolicySet` or `BoundPolicy`.
Mandatory admission filters offers before the callback; the callback chooses
among admitted routes and cannot weaken those requirements.

`FrameRouteOffer::output_schedule` carries the same guarantee for retained
video routes. `StagedEncode` supplies finish-first automatically, with unknown
wait retention, and uses the same policy before frame acceptance. Spool writes
are separate from final-destination waits.

The native JPEG adapter witness executes both schedules from generated strided
rows with real short writes. The owned row-source driver below executes this resolver; native total-retention
qualification remains separate.

### Owned row-source encoding

`zencodec_io::row_encode::encode(encoder, source, writer, policy, options,
pulse)` is the shared synchronous driver. `Source::description` fixes width,
height, descriptor and optional color context; `read_rows(first, PixelSliceMut,
pulse)` fills requested active samples with native stride support.
`Raster::new(PixelSlice)` preserves a complete raster's stride and color
context. The driver preserves context on each native row push and rejects
changed declarations before accepting those samples.
It snapshots acquisition facts before policy and revalidates declaration and
mechanism after policy, before each output drain, and before/after row reads.
A callback-induced change fails with `Refusal::SourceChanged`; already confirmed
appends remain in the failure report. This is boundary validation, not a lock
against concurrent mutation of externally shared source state.

`Options::new(storage, max_output_bytes)` admits row and staged payload
storage from a shared pool. `with_batch_rows` sets a nonzero request size;
pass `PolicySet` or `BoundPolicy` as the operation policy. Deprecated
`with_policy_set` requirements remain mandatory. Finish-first is the first
offer. It stages pending compressed chunks, completes/releases the encoder
and source, then writes. Live output requires actual streaming timing.
The final pending bytes go through finish before the last destination wait.
No row-level refusal silently falls back to allocating a whole raster.

`Completed` carries appended bytes, executed schedule and decision report.
Located `Failure` carries stage, confirmed appends, bounded decisions and the
original cause; formatting omits arbitrary backend payloads. Source, policy,
native, admission, cancellation and writer failures consume the operation.
Partial writes are never retried.

This replaces the private JPEG scheduling loop. `encode_to_writer` is now
deprecated; one-shot-only encoding remains `Encoder::encode` followed by writing
its output. Native workspace, source allocations and queue bookkeeping are
outside payload admission. A blocking OS operation cannot be interrupted by a
cooperative stop. Qualified native total-retention bounds remain separate work. This fixed-interpretation driver preserves supplied
context; it does not perform color conversion or metadata/privacy resolution.


`Options::with_disk_staging(file, replay_bytes)` explicitly replaces in-memory
compressed staging for finish-first execution. It takes ownership of a private
scratch file positioned at EOF, preserves its prefix, and uses the output-byte
limit as the spool payload cap. Replay admits a bounded chunk from the same
storage pool after the encoder and source are released. Construction seeks but
does not write. The caller owns permissions, path retention and deletion; file
aliases and concurrent modification are unsupported. Live selection leaves the
scratch unused. Spool I/O can block while native encoder state remains alive.

`OutputScheduleOffer::staging` exposes the actual `StagingFacts` through the
existing callback and selected report: memory/disk, current payload and limit.
A live offer has no staging. No new policy callback or automatic fallback is
introduced. `Stage::Replay` distinguishes spool reads/admission from destination
writes; late failures retain confirmed destination appends and concrete causes.


### Shared output completion and destinations

`zencodec::output_patch::{OutputFinish, Patch, PatchError}` owns the common
sans-I/O completion contract. `OutputFinish::validate(emitted_bytes)` checks the
entire ordered patch set against the final stream extent, including the tail,
before any final effect. `apply_to(&mut Vec<u8>)` preserves contents and length
on validation or reservation failure. Overlaps apply in list order. A zero-byte
patch is legal at the end; out-of-extent offsets and arithmetic overflow refuse.
External I/O remains non-transactional after successful preflight.

`zencodec_io::destination::{Destination, Forward, SeekBack}` binds writer access
and output origin. The trait is sealed; seek support cannot be claimed separately
from a `Write + Seek` writer. SeekBack offsets are relative to its captured
initial position, with checked addition and verification of the returned seek
position. Neither wrapper admits memory or authorizes encoder backpressure.

Media retains `container::{MuxFinish, Patch, PatchError}` and
`output::{Destination, Forward, SeekBack}` as aliases to these exact types.
Existing native muxers and consumers therefore use the shared implementation
without a migration. Raster negotiation and execution now use these shared types, as described below.
Bounded multi-pass source requests remain unimplemented.


### Negotiated raster seek-back

`Encoder::begin_output(width, height, descriptor, OutputAccess)` binds a framing
choice to actual destination access. Its default calls the existing forward
`begin`; it does not change quality or metadata settings. Patching implementations
must accept SeekBack, return `OutputTiming::with_patches()`, and complete through
`finish_output`. Legacy begin/finish paths must never silently lose required
patches. DynEncoder exposes both methods and the shared shim forwards overrides.

`finish_output` returns `OutputFinish<EncodeOutput>`: the tail retains format,
MIME, extensions and codec statistics. Container `OutputFinish` defaults to a
byte vector. Both validate with the same range checks. Existing encoders use a
provided finish adapter that transfers the original EncodeOutput without copying.

For callers, `row_encode::encode` remains the forward-writer convenience;
`encode_to_destination(..., SeekBack::new(writer)?, ...)` enables real seek-back.
An incompatible declared layout refuses before policy, row reads or output.
The driver preflights every final patch before tail/staged publication, rejects
unannounced patches, then writes the tail and applies patches in order. Successful
completion restores the final end position. `patched_bytes` reports confirmed
overwrites separately from `emitted` appends, including partial patch failure;
`Stage::Patch` covers seeks and overwrites. External writes are not transactional.

Qualification uses a private one-frame adapter over native AV1 and IVF. It
buffers the input raster explicitly, negotiates forward versus count-patched
IVF, preserves a destination prefix, and independently verifies decoded samples.
This is transport qualification, not a bounded/native streaming raster encoder.
Production raster codec adoption and repeated-input execution remain open.

### Legacy policy setter migration

`EncodeJob::with_policy`, `DecodeJob::with_policy`, `DynEncodeJob::set_policy`
and `DynDecodeJob::set_policy` are deprecated: they can silently ignore required
fields. Use `try_with_policy` (typed), `try_set_policy` (typed or erased), or
`EncodeRequest::with_policy` / `DecodeRequest::with_policy` with checked execution.
The operation request builders store requirements; execution reports rejection.
Deprecation does not change existing codec behavior, and legacy `CodecSet`
conveniences retain their previous best-effort semantics. Their presence is not
a guarantee that a requested policy was honored.

### Preset obligations at interpretation adoption

A provisional boundary may select `ContinueReading` while requirements remain
unresolved. `AdoptInterpretation` can immediately permit native sink output;
there is no guaranteed later conversion or publication question. The built-in
preset therefore refuses adoption if metadata retention, opaque-profile
exclusion, workspace bounds, extended-range preservation, output scheduling or
output-wait bounds remain unproven. A final descriptor is not evidence for those
separate obligations. A coupled adapter can prepare the relevant routes and
resolve their question before explicitly adopting the same held interpretation.
This refuses unsupported requirements; it does not turn discovery into a
whole-operation allocation cap. Mandatory requirements filter the offered routes
before custom callbacks run. Direct selection cannot bypass that filter.

### Workspace deadlines before native preparation

```rust,ignore
// Adapter entry: snapshot once, before creating native jobs or decoders.
let bound = policy.requirements();
legacy.check_workspace(None)?;
if let Some(bound) = bound {
    bound.check_workspace(None)?;
}
// Retain both snapshots for the later joint route decision.
// Only now may an unqualified native route begin preparation.
```

`PolicySet::check_workspace(Option<u64>) -> Result<(), UnmetRequirements>`
uses the same predicate and typed diagnostics as joint route admission. An
unknown bound refuses every configured workspace cap, including `u64::MAX`.
An adapter must check before preparation uses covered workspace: rejection at
the later policy callback cannot undo earlier native allocation. Managed row
storage is not a bound on codec/conversion workspace. A supplied qualified bound
must cover preparation and verification too, within the documented workspace
scope; it is not a whole-operation memory limit.

This check neither invokes the callback nor clears any requirement. Both legacy
and bound requirements remain mandatory. `UnmetRequirements` implements
`core::error::Error` so adapters can retain its named enum evidence through a
typed cause chain without inventing a question, ticket or selected offer.

The shared helper does not itself enforce an adapter's call order. Native
integration must prove refusal before job/decoder/conversion preparation and
preserve the existing final-boundary refusal. Likewise final pixel facts do
not prove a complete source metadata inventory: a publication owner must qualify
the metadata required by its treatment and reject relevant late changes before
releasing its output. See [controlled decoding](controlled-decoding.md).

## Encode declaration and preflight (review API)

```rust
EncodeDeclaration::new(width, height, descriptor)
    .with_color(current_color_context)
    .with_access(destination_access);

// Typed and erased jobs preserve native errors in the existing boxed envelope.
fn plan_encode(&self, declaration: &EncodeDeclaration<'_>)
    -> Result<EncodePlan, BoxedError>;

// Typed Encoder returns its own error; DynEncoder boxes it without stringifying.
fn begin_declared(&mut self, declaration: &EncodeDeclaration<'_>)
    -> Result<EncodePlan, Self::Error>;
```

Both hooks default to explicit `UnsupportedOperation::EncodePlanning`, without
constructing or invoking a legacy encoder. Planning cannot start native image
state or workers. A driver retains the exact job and declaration, admits the
route, starts only that job, and validates startup against the admitted plan
before reads or publication. The hooks themselves do not execute that driver.

`EncodePlan` separates `InputRequirement::{Rows, WholeRaster, Replay}` from
`OutputTiming` and `OutputPhase::{AnyPass, FinalPass}`. Rows can still buffer the
whole image internally. `SourcePasses` requires a positive range; finite budgets
compare its maximum, never just its minimum. Replay execution remains in the
qualified experiment until shared-driver integration.

`ColorCarriers` holds exact materialized ICC bytes and native CICP. `None` in the
plan means unreported evidence; `Some(ColorCarriers::default())` means confirmed
absence. Neither certifies PII removal. Startup comparison covers the complete
plan, including actual carrier bytes. No native memory bound is inferred.

The core preflight tests cover unsupported defaults, animation-only jobs, typed
and erased forwarding, no construction during planning, original error recovery,
current color/access binding, invalid lag/access and startup fact drift. Native
JPEG/PNG implementations and checked destination integration are separate gates.

### Execute a planned row job

```rust
zencodec_io::row_encode::encode_job(
    job, source, writer, resolver, options, &pulse,
)?;
// For a Destination that can overwrite earlier output:
zencodec_io::row_encode::encode_job_to_destination(
    job, source, destination, resolver, options, &pulse,
)?;
```

These adapter functions consume an already configured `EncodeJob`. They retain
it through planning and policy, recheck source interpretation and destination
access, reserve driver rows, then construct and start the encoder exactly once.
The current source color context reaches `begin_declared` before any header can
be generated. Startup-plan drift fails before source reads or output writes.
Whole-raster and replay input protocols refuse before construction; this driver
currently executes only the row protocol.

Policy refusal precedes driver row reservation and encoder construction. The
existing schedule policy still refuses unknown required workspace/wait bounds.
The default finish-first route stages compressed output and releases the encoder
before destination waits. Planning itself remains a codec contract, not an
enforced sandbox or a native allocation bound.

`Stage::Start` identifies construction, declared startup and startup-plan drift.
Original native errors, the selected policy record and confirmed output counts
remain available on failure. This is not yet the checked registry
`EncodeRequest`-to-destination convenience: it does not infer metadata or fidelity
requirements on the configured job. Existing raw-encoder entry points retain
their startup behavior.

### Encoder-driven source passes

```rust
use zencodec::encode::{EncodeRows, PullProgress, RowDemand, RowFillError};

// Adapter supplies samples into a native-owned, strided strip.
trait EncodeRows {
    fn fill(&mut self, demand: RowDemand<'_>) -> Result<(), RowFillError>;
}

// Typed Encoder and object-safe DynEncoder expose the same protocol.
fn pull_rows(&mut self, rows: &mut dyn EncodeRows, pulse: &dyn Pulse)
    -> Result<PullProgress, Self::Error>;
```

`begin_declared` binds geometry, interpretation and output access. A replay plan
supplies source-pass bounds before the driver reads pixels. Each `RowDemand`
contains a zero-based pass, final-pass marker, absolute first row and native
`PixelSliceMut`; its optional diagnostic label has no scheduling meaning. The
final-pass marker stays constant across every strip in a traversal.
The row loan ends when `fill` returns, so it cannot cross an async suspension.
The driver validates ordering, geometry, interpretation and any exact-depth
assertion and retains its original typed failure. `RowFillError` is only the
signal returned to the codec; swallowing that signal cannot authorize output.

`Filled` means exactly one successful fill; `Done` means no fill and all passes
completed. Both use the existing pending-output/consume/finish methods. The
provided default refuses before calling the source. The erased shim forwards
the native result without replacing its error. This method does not itself
admit replay, storage or publication policy, and does not implement async I/O.
The experimental IMI adapter uses the shared typed method. Production joint
policy, native retained preparation and native `Send` qualification remain
separate gates; the production row job driver still refuses replay input.

## Joint configured-job encoding route (review API)

```rust
fn resolve_encode_route<R: OperationPolicy + ?Sized>(
    declaration: &EncodeDeclaration<'_>, plan: &EncodePlan, evidence: EncodeEvidence,
    offers: &[EncodeRouteOffer], resolver: &mut R, preset: PolicySet, pulse: &dyn Pulse,
) -> Result<(usize, DecisionRecord), At<PolicyFailure>>;
```

One retained configured job supplies the declaration, plan and common
`EncodeEvidence`.
The plan is borrowed in `Question::EncodeRoute`; `DecisionContext::EncodeRoute`
copies only `EncodeRouteFacts` (pixels, evidence, input, timing, output phase).
`Selection::EncodeRoute` copies the chosen acquisition/output/evidence offer.
There are no input owners or ICC payloads in the report.

`EncodeEvidence::new(EncodeOffer)` retains the existing encoder offer and defaults
its `metadata: Option<MetadataTransfer>` and `retains_opaque_profile: Option<bool>`
to unknown. Supply this value once for the configured job. It is available through
`facts.evidence`, shared by all alternatives and retained in the bounded context.
`EncodeEvidence::unknown()` also leaves format, fidelity and resolution unknown
for generic retained jobs that cannot expose qualified configuration facts.
It supplies no exception to mandatory admission.

`EncodeRouteOffer::new(InputAcquisition, OutputScheduleOffer)` initializes replay,
workspace and range evidence to unknown. Metadata/profile facts are not route
fields. Acquisitions are
`Unknown`, `BorrowedRaster`, `DirectRows`, `MemorySnapshot` and `DiskReplay`.
Replay evidence distinguishes `Unknown`, `ImmutableBorrow`, `PrivateSnapshot`,
`VerifiedIdentity` and `VerifiedHash`. The first exact form requires an actual
held raster; the second requires a private raw memory snapshot. Source flags
alone never establish either. Structural compatibility is always checked,
including with `PolicySet::new()`; filtering preserves original selection indices
and incompatible choices never reach the callback.

`with_max_source_passes` counts external traversals: direct acquisition uses the
native maximum, staging uses one external pass even when internal replay has no
known maximum. Zero rejects all source reads. `require_exact_replay` needs no
replay evidence when the retained native plan proves at most one pass (`Rows`,
`WholeRaster`, or `Replay` with maximum one). Otherwise it refuses unknown, hash
and identity evidence. The exemption is derived from the native plan, never a
caller flag or the external staging traversal count.
`allow_raw_memory_staging(false)` excludes raw memory snapshots, not compressed
output stores. Other question domains keep
these obligations unknown; final boundary adoption refuses them. Existing
mandatory requirements apply unchanged, including strict metadata/workspace,
range and output-wait evidence. Extended range additionally requires confirmed
lossless fidelity, as on frame routes. Timing/FinalPass never prove retention.

The driver must check both requirement snapshots before covered preparation,
retain the selected job/source/reservations, and revalidate the complete startup
plan before reads or publication. Core admission does not enforce driver call
order or certify native output. Caller and adapter examples, migration scope,
evidence semantics and unrun validation recipes: [joint route contract](encoding-route.md).

Invalid retained plans refuse before any callback, even with an empty offer set:
`DecisionCode::InvalidOffers` retains the original `EncodePlanError` directly in
the cause chain. Incompatible alternatives under a valid plan remain individually
filtered via `PolicyRequirement::EncodeRoute`. One configured-job evidence value
supplies metadata/profile obligations for every route; there is no per-offer
metadata consistency check or consistency error type. Workspace/range remain
route-dependent. Source-pass and all other mandatory checks still apply to plans
proving a single native pass.

### Explicit encode orientation

`encode::EncodeOrientation::{PreserveStored, Bake}` is applied through
`EncodeJob::try_set_orientation_mode` (or its consuming builder), the matching
erased setter, or `operation::EncodeRequest::with_orientation_mode`. Defaults
refuse until a codec implements the requirement; omitting it preserves existing
behavior. See [the contract and native limits](encode-orientation.md).
