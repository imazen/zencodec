# Opaque data and decisions during transcoding

Investigation, 2026-10-25. zencodec baseline `72b0a553`; zencodecs source
reviewed at zenpipe `04c874a175883ab2a3d17e6267fcac599c22a50a`.
This extends the [policy proposal](policy-resolution.md), without changing a
public interface. Executable qualification is recorded separately below.

## Opaque is an unresolved obligation

A required privacy policy must not interpret inability to inspect as evidence
that data is clean. This includes invalid XML, unavailable parsers, unsupported
RDF shapes, unknown namespaces, compressed packets whose expansion exceeds the
budget, private tags, and uninspected nested objects. Parsing successfully proves
syntax, not that every retained field or value satisfies the privacy policy.

For each output carrier, admit one of these actions:

| Evidence and obligations | Admissible action |
|---|---|
| A qualified check establishes the requested exclusions for the exact bytes | Preserve those bytes within that scope. |
| A supported typed subset contains everything required | Generate fresh metadata from validated values; omit unknown content. |
| The carrier is safely removable and nothing requires its contents | Drop the whole carrier, recording the omission and reason. |
| More bounded inspection can resolve the question | Inspect using an available service, within admitted limits, then decide again from new facts. |
| Removal conflicts with required attribution, rendering, reconstruction or another effect | Refuse unless a separately permitted, qualified alternative satisfies all requirements. |

There is no "pass through because parsing failed" answer under a required
exclusion. An application can explicitly change its requested scope or grant a
specific exception, but that output must not be reported as satisfying the former
strict request. An exception belongs to the request, not an unchecked callback
response. Errors distinguish unavailable inspection, malformed data, limits,
unknown semantics and contradictory requirements.

"PII" needs a defined scope. EXIF rights and arbitrary text can themselves contain
names or contact details. ICC profiles can contain identifying text/private tags.
Thumbnails, depth/auxiliary images, attachments and media tracks need their own
dispositions. Faces, voices, visible text and covert channels in image/audio samples
are outside a metadata-exclusion contract. Prefer a named, versioned metadata
allowlist with explicit retained attribution over an unqualified anonymity promise.

Freshly generated interpretation metadata must preserve actual sample meaning.
Dropping an unknown ICC profile and labeling the pixels sRGB is not a valid privacy
fallback. A qualified normalization or explicit color conversion may be available;
otherwise preserving rendering and excluding uninspectable metadata can conflict.
Likewise, a byte-exact reconstruction promise cannot coexist with removing private
bytes that would be recovered from the preserved original representation.

Current `Metadata::filtered` already drops all XMP under `Web`, drops opaque EXIF
under partial filtering, and documents its scope limitations. `PreserveExact`
can retain opaque packets. `Web` retains rights and nonredundant ICC; even
`ColorAndRotation` retains ICC. None certifies a whole encoded file free of PII.
`Services::rewrite_xmp` defaults to refusal. `normalize_icc` returning `None`
means unchanged/unrecognized, not safe-for-privacy. Reuse these mechanisms, but
do not treat their absence or successful parsing as a privacy certificate.

## New facts are decision boundaries

An operation can learn interpretation or disclosure facts midway through input.
Initial planning alone is insufficient. Distinguish pending facts, supported
facts, facts whose relevant scope is closed, and facts that cannot be established.
Unknown alpha/HDR/metadata is not `false`; a ready color profile is not necessarily
a promise that later information cannot refine it.

The lifecycle is discover -> suspend affected work -> construct admissible
alternatives -> resolve -> validate/reserve -> resume. Repeat when a relevant fact
revision invalidates a decision. Identical notifications do not require another
decision. Rows, byte arrivals and writer stalls by themselves are not policy
questions. The ordinary transcode call drives this loop with its configured
resolver; callers do not coordinate decoder/encoder capability queries.

A decision receipt binds operation, source identity, fact generation, affected
stage and selected contract. A new profile, late supplement, changed track epoch
or replaced source invalidates affected receipts. Callback failure/cancellation
must leave the operation suspended or terminal, never implicitly authorized.
Async application consultation can live in a driver that owns a suspended task;
the core resolver need not require an async runtime or hold global resource locks.

Track separate irreversible boundaries: input consumed without replay, sample
conversion with the original discarded, encoder configuration committed, HTTP or
container headers published, bytes published, and media segments/frames exposed.
Zero published body bytes does not mean that the target format is still mutable:
HTTP headers alone may commit it. A private temporary file can permit rewrites
which a network writer cannot. Returned frames keep their original interpretation.

The strict disclosure invariant applies before every outward write: prohibited
data must not escape and then be excused by a later error. This does not require
buffering an entire input in every case. A qualified encoder can generate only
allowed carriers and discard unknown descriptive input as it arrives. Conversely,
unknown rendering dependencies or mandatory preservation may require bounded
lookahead, ranges, raw/encoded staging, replay, a different permitted route, or
refusal. All staging still needs admission; "wait for metadata" is not permission
to retain an arbitrarily large raster.

## Current streaming seam

[`MetadataUpdate`](../src/traits/incremental.rs) has HeaderReady, ColorReady and
Updated, with color explicitly available **or refined**. The event contract says
new informational variants can be ignored. Current slice/read-at/async drivers
step past metadata events, and a step can already call the sink before returning
its event. [`EncoderSink::begin`](../zencodec-io/src/encoder_sink.rs) starts the
encoder; the zencodecs TranscodeSink can immediately drain its header to a writer.

Consequently, adding an ordinary observer after `step` does not establish a
decision barrier. A policy-aware protocol must suspend before affected sink,
conversion or publication effects and require an explicit resume decision. It
cannot hide mandatory acknowledgement in a new ignorable Event variant or pretend
a byte Request is a policy question. Existing terminal SinkError is refusal, not
a resumable pause. A compatibility wrapper may stage work, but must disclose and
admit that storage; it cannot promise free streaming compatibility.

## zencodecs decision space

The resolver belongs across the transcode operation. Low-level codecs expose
facts and execution mechanisms; adapters provide storage, editing, color services
and publication; the facade assembles feasible routes and invokes the one resolver.

| Decision | Required facts and constraints |
|---|---|
| Format/backend and fallback | Allowed codecs, actual source interpretation, target acceptance, replay and commitment state. A failed decoder is not permission to retry after irreversible work. |
| Route | Copy, metadata-only rewrite, coefficient reuse/reconstruction, pixel decode/encode, or media remux/transcode. Each route has different disclosure and fidelity coverage. |
| Interpretation | Color/ICC/CICP, precision, alpha/matte, orientation, HDR rendering and gain-map consistency. Descriptive overrides cannot relabel pixels. |
| Content | Required images/pages/levels/regions, animation frames, tracks, timestamps, auxiliary images and attachments. Optional omission must be explicit. |
| Metadata | Known and opaque carriers, nested data, parse/service evidence, required attribution and scoped exclusions. |
| Resources and progression | Qualified bounds/accounting, pending knowledge, lookahead/range access, staging, concurrency, output waiting and cancellation. |
| Publication | Whether representation/headers/segments are fixed, what can be rewritten, and what remains required before completion. |

On the reviewed zencodecs main, `TranscodeOptions` separates metadata retention,
supplements, matte, orientation and color. `transcode` materializes pixels and
then builds an EncodeRequest; metadata probing failure is converted to an empty
Metadata record. That cannot stand for successful required metadata inspection or
preservation. `SupplementPolicy::Preserve` explicitly permits unsupported
supplement omission. Coefficient/JPEG-reconstruction routes have their own entry
points and need the same operation obligations. The streaming TranscodeSink
accepts an already chosen encoder and can publish during begin. These are concrete
integration seams, not evidence that current callers already have a hard PII or
late-decision guarantee.

Related unmerged work already exists: [zenpipe PR 84](https://github.com/imazen/zenpipe/pull/84)
(`249cf899`, gain-map metadata retention) and
[PR 87](https://github.com/imazen/zenpipe/pull/87) (`aa21cba0`, bounded metadata-only
scrubbing and optional runtime services). Their declared scope preserves rendering,
supports selected containers and excludes arbitrary ICC sanitization. Reuse and
qualify that work; their recorded test claims are not a fresh local rerun here.

## Qualification status

The private [disclosure adapter](../perf/io-validation/src/policy_disclosure.rs)
uses `zenmeta::xmp::Packet` for bounded XML/RDF parsing. Its versioned allowlist
covers **one field: TIFF orientation, values 1 through 8**. It can preserve an
exact canonical packet, regenerate the supported value without other fields,
drop a removable optional packet, or refuse. The approved result owns or borrows
the exact outgoing bytes, so a receipt cannot authorize a substituted packet.

`just io-policy-disclosure-check` passes the carrier controls and existing resolver
regressions, with Clippy warnings denied. Cases include unknown namespaces and
free text, comments and blobs, unavailable inspection, invalid UTF-8/XML, DTDs,
byte/depth limits, duplicate/unsupported RDF values, required reconstruction,
opaque ICC and nested/critical objects. Five route labels exercise the same
disclosure contract; they do **not** implement five transcode pipelines. The
orientation read-back uses the same parser, not an independent XML oracle.

The test/build process receipt was `rc=0 16s | peak-RSS 0.45GiB | min-avail
15949MiB | peak-load 2.15`. This is a process measurement, not a codec or parser
memory bound. Byte/node/depth limits do not establish whole-operation heap usage.

Suspended-stream controls are being added next. No production state-machine or
public policy interface has changed in this investigation.
