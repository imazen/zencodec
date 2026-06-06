# Correctness model: color, orientation, and metadata

Three things in an encode are *correctness* signals, not preferences: the color
carrier (ICC vs CICP), the display orientation, and which metadata is retained.
Get any of them wrong and the output is wrong — colors shift, the image displays
sideways or double-rotated, or private data the caller asked to strip ships
anyway. This document describes how zencodec keeps those decisions in one place
so individual codecs can't quietly diverge, and how to verify a codec honors the
contract.

The guiding idea is a pit of success: the framework resolves each signal to a
plain, embed-ready value *before* the codec runs, and the codec only embeds what
it is handed. A codec author who does the obvious thing gets correct behavior;
getting it wrong takes effort.

## The signals are stream-scoped, the feeding mode is orthogonal

A signal is decided once per encode, on the job, before any pixels move:

- `EncodeJob::with_policy(EncodePolicy)` — the color-carrier emission policy
  (`ColorEmitPolicy`), resolved against the target's capabilities by
  `resolve_color_emit`.
- `EncodeJob::with_metadata_policy(meta, MetadataPolicy)` — metadata retention,
  applied by `Metadata::filtered` before the record reaches the codec.
- The orientation lives on `Metadata::orientation` (the authoritative field) and,
  redundantly, in the EXIF blob's `0x0112` tag. `filtered` reconciles the two.

None of this depends on *how* pixels are fed. A codec may take the image in one
shot (`encode`), incrementally (`push_rows` + `finish`), by pull (`encode_from`),
or as animation frames (`push_frame`); on the decode side, one-shot (`decode`),
streaming (`streaming_decoder`), push-sink (`push_decoder`), or animation. The
feeding mode is a performance/streaming concern. The correctness signals are set
on the job and are identical across every mode — which is exactly why the testkit
checks that all feeding modes produce identical output (see below). If two paths
for the same operation produce different pixels, that is a bug in one of them,
not a tradeoff.

## Metadata retention: one primitive, one safe wrapper

Retention is a privacy decision, so the API makes it impossible to ship metadata
without choosing one — at compile time, without a breaking change.

`EncodeJob::with_metadata(meta)` is the storage *primitive*. Every codec
implements it: store the bytes, embed what the format supports at encode time.
It is marked `#[deprecated]`. The deprecation is aimed at *callers*: the compiler
warns at any call site that hands a codec metadata without naming a retention
policy. Implementing the method does not warn (Rust's `#[deprecated]` fires at
call sites, not impl sites), so codecs implement it normally.

`EncodeJob::with_metadata_policy(meta, policy)` is the blessed path and the one
without a warning. It is a provided method: it calls `meta.filtered(&policy)` and
hands the result to `with_metadata`. Because the filter runs first, the codec
only ever receives what the policy kept. A codec cannot leak what it never saw.

| Audience | Calls | Implements | Sees a warning? |
|---|---|---|---|
| Codec author | — | `with_metadata` (primitive) | No |
| Pipeline / app | `with_metadata_policy` | — | No |
| Pipeline / app | `with_metadata` (no policy) | — | Yes — pick a policy |

`MetadataPolicy` has no `Default`; callers name one. `Web` is the privacy-safe
choice for publishing (keeps ICC, EXIF orientation + rights, and color signaling;
drops GPS, timestamps, camera identity, thumbnail, and all XMP). `PreserveExact`
is a verbatim round trip. The per-variant promises and the edges where they don't
hold are documented on [`MetadataPolicy`](../src/metadata.rs) under "Delivery
exceptions" — the short version: a partial policy fails *safe* on an unparseable
or oversized EXIF blob (it drops the whole blob rather than risk leaking), and
the consistency between CICP/HDR signaling and a gain map is the caller's job
because `filtered` can't see the gain map.

## Orientation: one authority, reconciled

Orientation can be expressed three ways — a container field, the EXIF `0x0112`
tag, and the pixels themselves (baked upright). The hazard is double-application:
a decoder bakes the rotation into the pixels and sets the field to `Identity`, but
the EXIF tag still says `Rotate90`, so a downstream consumer that re-reads the tag
rotates a second time.

`Metadata::orientation` is the single authority. `filtered` reconciles the
embedded EXIF tag to match it (rewriting `0x0112` in place via
`helpers::set_exif_orientation` only when they disagree, so the common case keeps
the zero-copy `Arc` clone). A codec embeds the field's value; it does not invent
orientation from a second source.

## Color emission: resolve, then embed

`EncodePolicy.color` (a `ColorEmitPolicy`) says how to carry color — prefer
compact CICP code points, keep the ICC bytes, or a custom mix.
`resolve_color_emit(&SourceColor, &EncodeCapabilities, ColorEmitPolicy)` turns
that into a concrete `ColorEmitPlan` (`{ cicp, icc: IccDisposition }`) against the
target format's capabilities — no CMS, no codec dependency, never a silent ICC
drop. The codec writes the carriers the plan names. The design and the rejected
alternatives are in [color-emit-model.md](color-emit-model.md).

The coarse `EncodePolicy.embed_icc/embed_exif/embed_xmp` flags remain as a
best-effort whole-channel gate, but they are not the retention control — they
no-op on a codec that doesn't implement `with_policy`. For privacy, use a
`MetadataPolicy`.

## Verifying a codec: zencodec-testkit

The contract above is only worth anything if codecs actually honor it. The
[`zencodec-testkit`](../zencodec-testkit) crate is how a codec proves it does. A
codec crate adds it as a `dev-dependency` and runs the checks against its own
`EncoderConfig` / `DecoderConfig`:

- `check_metadata_no_leak` — encodes with rich metadata (GPS + thumbnail + camera
  + copyright + XMP + ICC) under several policies, decodes, and re-parses the
  embedded EXIF to assert that anything the policy dropped is actually gone. It is
  a subset check (decoded ⊆ filtered), so a codec that supports fewer channels
  still passes — it may drop more, never add back. This is the privacy guarantee,
  tested end to end rather than asserted on a struct field.
- `check_cross_path_pixel_equivalence` — runs every advertised encode and decode
  feeding mode and diffs the results byte-for-byte. This is what catches
  buffered-vs-streaming divergence (tile-boundary handling, edge padding, rounding
  differences between a one-shot path and an incremental one).
- `check_orientation_roundtrip` — confirms an orientation survives a keeping
  policy exactly once, catching both loss and double-application.
- `check_capability_honesty` — comprehensive and bidirectional: every declared
  capability (the encode paths `push_rows`/`encode_from`/animation, the decode
  paths streaming/animation, the `lossless` knob, `cheap_probe`, the
  `icc`/`exif`/`xmp`/`cicp` metadata channels, and `native_alpha`) must actually
  work, and every *undeclared* optional path must decline with
  `UnsupportedOperation` rather than panicking or silently succeeding. All
  violations are collected and reported together. (Cancellation and the lossy flag
  are out of scope: cancellation timing isn't reliably testable on bounded inputs,
  and lossy-vs-lossless output isn't observable from the bitstream.)

The testkit ships two codecs the checks are validated against in its own tests, so
the harness is known-good before you point it at a real codec: a faithful
`reference` (round-trips pixels *and* metadata, declares and honors every
capability) and a `minimal` one (one-shot only, every optional capability declared
false) that exercises the false-direction branches.

## Known hazards the testkit is meant to catch

These are the failure modes that motivated the model. They are not hypothetical;
they are the shapes of bug that recur across codecs.

- **Buffered vs streaming pixel divergence.** Any format with block structure
  (AVIF/AV1 tiles, JPEG MCUs) or a channel reorder (WebP BGRA swizzle) risks the
  incremental path producing different pixels at boundaries than the one-shot
  path. `check_cross_path_pixel_equivalence` diffs them.
- **Encode-side metadata ignored.** A codec whose encode path never calls
  `with_metadata`/`with_metadata_policy` silently drops everything — orientation,
  color profile, rights. Under a `PreserveExact` policy the testkit's positive
  round-trip expectations (and a codec's own tests) surface this; the privacy
  check alone won't, because dropping everything never *leaks*.
- **TIFF-family multi-representation orientation.** A TIFF codec can carry
  orientation in the IFD `0x0112` tag *and* bake it into the stored pixels *and*
  surface it as a container field. All three must agree, and a re-encode must not
  reintroduce a tag the pixels already satisfy. `check_orientation_roundtrip`
  exercises the round trip; a codec that represents orientation in more than one
  place should run it under every policy. (Reconciliation logic lives in
  `Metadata::filtered`; a codec that bypasses `filtered` is on its own.)
- **Color carrier clobbered at the codec.** A codec that overrides the resolved
  color carrier from a pixel descriptor (rather than embedding the
  `ColorEmitPlan`) can emit signaling that disagrees with the pixels. Embed the
  plan; do not re-derive.

If you add a codec to the zen family, wire up the testkit before wiring up the
pipeline. The checks are cheap to run and the bugs they catch are expensive to
ship.
