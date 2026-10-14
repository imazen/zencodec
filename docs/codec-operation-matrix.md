# Codec operation matrix and adversarial contract audit

Source audit: 2026-10-13, zencodec `1a91756e`. This is a proposed test model,
not a completed exhaustive run. No production API or existing assertion changes
are made by this document.

## What exists

- [Contract coverage](testkit-contract-coverage.md): fixture integrations for
  17 of 18 format adapters. AVIF, hardware-backend runtime coverage, allocation-failure
  injection, and animation sink timing remain explicit gaps. This is not all-pairs
  transcode coverage.
- [Testkit](../zencodec-testkit/src/lib.rs): capability, fidelity, metadata,
  color, orientation, gain-map, path-parity, error, and inventory checks.
  `check_all` uses fixed fixtures and does not call every opt-in helper.
- [Fixture-driven checks](../zencodec-testkit/src/contracts.rs): timing,
  checked-policy admission, inventory limits, and rewrite plans. Consumer tests
  supply expected results; there is no complete backend/configuration registry.
- [CodecSet tests](../zencodec-testkit/tests/codec_set.rs): reference-codec
  dispatch, metadata transcode and precision rejection. They do not run every
  real decoder against every real encoder.
- [zencodecs metadata conformance](https://github.com/imazen/zenpipe/blob/main/zencodecs/tests/metadata_conformance.rs)
  already contains a support table, an observational `print_support_matrix`,
  and ordered codec-pair copyright/orientation tests. Reuse these fixtures and
  observations; the printing test always passes and is not an acceptance oracle.
- [Gain-map routes](https://github.com/imazen/zenpipe/blob/main/zencodecs/tests/cross_codec_gainmap.rs)
  cover selected JPEG, AVIF and JXL directions, not every route.
- [Media draft](https://github.com/imazen/zencodec/blob/feat/media-foundation/media/docs/IMPLEMENTATION.md)
  records animation and high-precision route experiments. It is unmerged and was
  not rerun in this audit. Reuse its image fixtures and route enumeration without
  making video or the draft media crate a prerequisite.

## Tables to generate from one test registry

A format name is insufficient: each row identifies adapter, backend, relevant
configuration, Cargo feature set, target, and exact source revision. A test-only
registry should generate both execution and documentation. Keep expected contract
outcomes separate from capability bits reported by the implementation.

| Table | Row dimensions | Required observation |
|---|---|---|
| Decode | Fixture, decoder configuration, output request, feeding/dispatch path, policy, limits | Admission, pixels, descriptor/context, orientation, sequence, components, returned metadata, typed errors |
| Encode | Pixel fixture, encoder configuration, feeding/dispatch path, fidelity, metadata/color/timing requirements, limits | Admission, emitted carriers and samples, actual frame timing, error and state after rejection |
| Transcode | Source fixture and decoder configuration to destination encoder configuration, explicit transformations, policies | Source interpretation, every intentional change, destination interpretation, retained/dropped components |
| Inspect/rewrite | Carrier fixture, strictness, inventory feature state, rewrite request, services, limits | Structural interpretation, retained payload, output plan and provenance, resource bounds, reader compatibility |
| State transitions | Accepted operation, rejected operation, accepted operation, finish/EOF/cancellation | No prohibited mutation, no lost frame/rounding phase, correct terminal state and partial-output semantics |

Maintain distinct expected outcomes: must accept with specified observations;
must reject with a specified cause; may recover under a named compatibility rule;
requires an explicit conversion/selection; unavailable in this configuration.
Report missing execution and unresolved expectations separately. Neither counts
as a pass. Expected rejection is a tested cell, not a skipped cell.

Every expectation also names its authority: current documented law, accepted
compatibility fixture, format rule, or proposed next-release law. A current API
that cannot express a proposed requirement gets an explicit API-gap result.
Never derive expected behavior solely from the codec's advertised capabilities.

## Finite exhaustiveness and unbounded inputs

Exhaust all registered source/destination pairs and enumerated configurations,
all finite policy states, and every applicable API path. Enumerate policy state
spaces mechanically rather than maintaining a handwritten list. Existing tests
cover all ICC/EXIF/XMP tri-state combinations on the reference codec; they do not
establish all-field interactions on every real decoder.

For dimensions, payload bytes, profiles, clocks and malformed inputs, exhaustive
coverage of all possibilities is not a defensible claim. Use semantic partitions,
exact boundaries, property generation, a real-file corpus and fuzzing. For each
numeric limit test below/at/above the boundary; include overflow and zero cases.
Pairwise generation is useful for broad interaction discovery, but cannot replace
full combinations of known interacting correctness fields.

Important partitions include odd/tiny dimensions and tile/MCU/strip boundaries;
tight and padded stride; integer/floating precision; alpha and hidden RGB; gray,
RGB and supported native layouts; ICC/CICP agreement and conflict; all orientations;
native HDR and gain-map components; still/animation/pages; duplicate, malformed,
unknown and oversized carriers; strict and compatibility parsing; incremental
input splits; cancellation at each poll; final-byte and cumulative limits.

Keep sample equality, displayed-image equivalence, compressed-payload identity,
metadata byte identity, semantic metadata equality, and privacy exclusion as
separate assertions. Lossy coding need not change the samples of every fixture;
lossless claims must specify their sample domain. Explicit conversion changes the
comparison oracle. Never turn wrong pixels into an allowed tolerance to satisfy
an unrelated test.

## Source-confirmed weaknesses in the current harness

These are observations of implementation, not newly reproduced codec defects.

| Finding | Consequence | Proposed test improvement |
|---|---|---|
| `check_animation_timing` treats every error as acceptable for a rejected case | An internal error can satisfy an expected timing refusal | Carry expected typed causes per case; preserve atomicity checks |
| Timing helper supplies the same pixels for every frame | It checks counts/durations and path parity but cannot distinguish reordered equal frames or agreement on wrong pixels | Accept distinct frame fixtures and independent expected frame content |
| `check_decode_truncation_series` rejects successful prefixes at or below one quarter of total file length, tolerating later success | A complete image with a large trailer can be rejected; lost required payload late in a file can pass | Specify meaningful payload boundaries and compatibility expectations per fixture; compare accepted output to its oracle |
| Metadata honesty is observed through a paired decoder, with unreadable channels excluded | A carrier can be missing, altered or leaked without being observed by that decoder | Add independent wire inspection and required presence/content assertions |
| `check_metadata_no_leak` checks a subset relation | Dropping all metadata can satisfy privacy while losing required display information | Keep privacy and required-delivery checks independent; combine them when both apply |
| Static capability checks use one fixture/configuration and reject undeclared success | They cannot express content-dependent losslessness or backend-dependent capability fully | Register configurations and fixture domains; retain truthful static discovery while designing operation admission |
| Color-context consistency allows absence | All paths can agree on missing context | Apply the existing positive attachment check where required, plus independent interpretation checks |

Changing an existing acceptance expectation still requires explicit review. The
permission to break testkit APIs does not itself approve weakening assertions.
Add a reproducer and distinguish a wrong implementation from an incorrect law
before changing either. New negative tests should exercise buggy adapters that
ignore a policy, misclassify an error, drop a carrier, truncate precision, relabel
color, advance state on rejection or accept an over-budget plan. Also include
valid recovery fixtures to detect laws that reject too much.

## Current revision versus next breaking revision

| Area | Current revision (0.1.27 candidate) | Next breaking target |
|---|---|---|
| Decode compatibility | Accept bounded, unambiguous real-world recovery where supported; retain explicit strict behavior and regression fixtures | Separate compatibility mode, extraction, and operation/content admission |
| Metadata access | Keep useful orientation/color/ISO gain-map access without zenmeta; specialists use zenmeta for editing/analysis | Remove deprecated editing implementations from core after migration; retain core request/result contracts |
| Policy requirements | Checked setters accept supported fields or reject atomically; legacy published setters remain best-effort | Make required operations explicit and return delivery outcomes; keep quality/effort hints separate |
| Fidelity | Do not turn `with_fidelity` into a new fallible contract silently; test actual samples whenever losslessness is reported | Validate requirements against actual pixels/configuration, including content-dependent cases |
| Color/orientation | Test current pixels and labels together; prohibit silent relabeling, double rotation and unrequested conversion | Final pixel context is authoritative, source provenance separate; report required conversion or unsupported signaling |
| Animation | Ordinary quantization reports actual timing; explicit exact mode rejects unrepresentable timing without corrupting state | Rational timing canonical, frame-aware sink results, explicit zero-duration and total-play semantics |
| Transcode | Current `CodecSet::transcode` rejects known sequences/auxiliaries and descriptor downgrades; fidelity and carrier delivery remain best-effort | Explicit handling of sequences/components and required outcomes; preserve ergonomic still-image entry points |
| Inventory | Default-off per-codec walker with callable typed unsupported behavior; no zenpipe dependency | Keep independent inspection; improve interpretation sharing without requiring decoding |

The current `CodecSet::transcode` implementation trusts `probe_full` for sequence
and auxiliary detection and exact descriptor membership for target admission.
Adversarial tests should include probe/decode disagreement and independently
unsupported color, alpha, precision and carrier combinations. Descriptor membership
alone does not prove required fidelity or realizable metadata delivery. This audit
does not claim an actual decoder currently bypasses those guards.

Do not publish an avoidable ambiguity simply to reserve its repair for a breaking
release: unpublished surfaces may still be tightened through reviewed PRs. Preserve
already-published signatures and documented compatibility until their migration.
See the [breaking queue](../CHANGELOG.md#queued-breaking-changes) and
[published API audit](zencodec-contract-audit-2026-10-13.md).

## Implementation order and ownership

1. Build the test-only configuration/fixture/expectation registry and explicit
   case/result records. Port existing metadata support rows and fixtures first.
2. Generate all ordered decode-to-encode pairs, including expected refusals;
   run existing per-codec helpers against named configurations.
3. Add typed timing rejection and distinct frame fixtures, then structural
   truncation oracles. Review changes to existing expectations separately.
4. Add independent wire readers and mutation checks, then corpus/fuzz expansion,
   allocation-failure testing, and the required target/feature/backend matrix.
5. Run current laws as release gates and proposed next-release laws as an explicit
   gap report until the corresponding production contracts exist.

Reusable laws belong in zencodec-testkit. Codec-specific fixtures and native
expectations stay with their codec. The all-codec executor belongs in a separate
test/tool package so heavy dependencies and independent readers do not enter
zencodec, zenmeta, or normal application builds. It can use CodecSet directly;
neither zenpipe nor zencodecs is required for the reusable tests. Reuse existing
zencodecs fixtures through extraction rather than recreating them. No new
production capability-matrix API or mandatory production crate is proposed.

Release evidence should name the enumerated scope and count passed, expected
refusal, unexpected acceptance, unexpected rejection, wrong output, API gap,
unavailable configuration and unexecuted cases separately. A green run over
selected fixtures is not a statement about all files or all configurations.
