# Code-first reader study, 2026-10-27

Status: in progress. No end-to-end success rate is available.

Two fresh SWE-2 Max sessions received frozen steps from the code-first site at
`7342ae3e`, first visible, then expanded, with compile feedback between attempts.
The consumer track covers registration, file I/O, conversion and transcode policy.
The adapter/implementor track covers configs/jobs, preflight, input and output.
Attempts, diagnostics and exposure order are retained separately. Automatically
injected project rules and the coordinator-supplied compatibility pin are recorded
as deviations: this is not a strictly blind study or a human usability measure.

## Independently checked observations

| Observation | Evidence | Action |
|---|---|---|
| Hidden imports lead to incorrect crate guesses | Consumer attempt 1: E0432 for JPEG/PNG configs at the zencodec root. Implementor attempt 1: E0432 for EncoderConfig/PixelDescriptor, E0603 for root EncodeCapabilities. | Show source-extracted imports without expanding the whole fixture. |
| Signature cards look complete but omit required methods | The encoder config card selected only encode/job. Required format/supported_descriptors were absent. There is no missing supertrait. | Identify partial selections; extract a required-method checklist and show the config identity methods. |
| Send registration sounds like output transport | Reader guessed with_send_encoder sends bytes and inferred no with_encoder sibling. Source contains both methods; the card omitted one. | Show both; explain thread-movable jobs versus destination writes. Do not rename based on the false absence premise. |
| No reproducible consumer dependency setup | Expanded imports identify zencodecs but provide no revision. The reader's zenpipe main at 04c874a1 fails dependency resolution on yanked heic ^0.2.0. | A frozen, independently compiled consumer project is still needed. This main-branch attempt is not evidence that every pinned harness fails. |
| Internal fixture helpers block codec reproduction | minimal.rs imports crate::reference helpers, and the minimal module is cfg(test), pub(crate). | Label the internal fixture and expose error construction; a self-contained implementor walkthrough remains needed. |
| Error and pixel vocabulary invite guesses | Implementor attempt 2 fails on guessed PixelSlice::height/bytes, UnsupportedOperation::PixelFormatConversion, bare-error .at(), and a boxed-error source conversion. | Separate Rust/compiler repairs from API findings. Add pixel row/stride and located-error context before proposing aliases. |

The first two setup attempts in each lane failed. These are intermediate attempts,
not the final results of all assigned tasks. Runtime outcomes and independent
replays will be added when available.

## Changes and verification

The import/signature fixes landed in `6f98ca41`; no existing assertion was
weakened. Source checks and browser coverage passed: all source scopes, exact
full-example copy, independent context expansion, signature panels, keyboard
navigation and 390/768/1440 px layouts. The readers continue against their frozen
old page. A revised-page study requires fresh sessions.

Test resources (including browser checks, not codec performance):
`rc=0 32s | peak-RSS 0.21GiB | min-avail 8250MiB | peak-load 0.69`.

Reader build receipts (compilation resources, not codec performance):

| Attempt | Exit | Time | Peak RSS | Min available | Peak load |
|---|---:|---:|---:|---:|---:|
| Consumer 1 | 101 | 5s | 0.30 GiB | 8604 MiB | 1.23 |
| Consumer 2 | 101 | 5s | 0.28 GiB | 8979 MiB | 0.75 |
| Implementor 1 | 101 | 2s | 0.23 GiB | 8753 MiB | 0.95 |
| Implementor 2 | 101 | 2s | 0.24 GiB | 8858 MiB | 0.38 |

Related tracking: [consumer ergonomics and contracts](https://github.com/imazen/zencodec/issues/159).

## Follow-up findings from repaired runtime attempts

The implementor passed its source-assisted roundtrip/refusal and preflight
controls in attempt 12. This is 2 of 4 assigned implementor tasks reaching runtime
at this observation; input and output controls remain pending. The consumer's
first control run only demonstrated garbage refusal: the purported valid fixture
was also unrecognized, despite a zero process exit. It does not count as a
positive roundtrip. The coordinator then supplied the exact-revision
[testkit caller examples](https://github.com/imazen/zencodec/blob/7342ae3eeddb4ddd21d88b792ee42c3db3ccb5e7/zencodec-testkit/tests/usage.rs)
as a recorded rescue. Resulting control successes must be reported separately.

Further independently checked concerns:

- `EncodePlan::validate(declaration)` checks zero geometry, lag, patch access and
  input protocol constraints. It does not retain or compare the original
  declaration. `validate_started` compares the reported plan facts; adapters
  still retain the same job/declaration. A clearer name or a bound preparation
  object is a design candidate, not an implemented guarantee.
- `EncodeJob::Enc` and `DecodeJob::Dec` intentionally lack the executor trait
  bounds. Typed generic callers add the capability bounds they use; a caller
  accepting the corresponding dyn config avoids repeating them. Blanket dyn
  implementations still have bounds: not every arbitrary EncoderConfig can be
  assumed to satisfy them.
- Output accessors were missing from the focused cards: encoded bytes borrow
  through `data()` or move through `into_vec()`; decoded `pixels()` returns a
  borrowed PixelSlice, while `into_buffer()` moves the PixelBuffer. The cards now
  show these signatures, plus Outcome output/report access.
- The fixture's `native_plan` is codec-owned test code, not a library function.
  Its generic name and whole-file expansion obscure that boundary.

One independent source check found a separate real rendering bug: array-type
semicolons truncated the pinned YCbCr constructor signature. `8f182cc6` fixes
array/const-generic parsing and recaptures the same revision and source hashes.
This was not a reader runtime failure.

Some initial reader interpretations were incorrect. Required methods do not
live on an omitted supertrait, and the selected default methods are not from a
different revision. Released 0.1.26 has `encode::EncoderConfig`; an unavailable
root import does not establish that the trait is absent. These corrections
matter when deciding whether to revise the API or its presentation.

## Clean checkout control

A public-source archive of `c39270dc` was unpacked outside workspace path
overrides. It contains no committed root Cargo.lock; Cargo resolved the published
dependencies on 2026-10-27. `cargo test -p zencodec-testkit --lib --test usage`
passed the internal fixture tests and caller examples, including conversion.
The site links this workspace manifest, shared helpers and command beside the
encoder/decoder implementor flows. This is a core fixture recipe, not a native
JPEG/PNG bootstrap or a reader first-attempt result.

Receipt: `rc=0 1s | peak-RSS 0.28GiB | min-avail 7341MiB | peak-load 0.65`.
The final browser run with visible imports, required signatures, output accessors
and corrected YCbCr signature passed existing desktop/mobile/clipboard gates:
`rc=0 36s | peak-RSS 0.30GiB | min-avail 6526MiB | peak-load 1.11`.

## Adapter card follow-through, 2026-10-28

The implementor's later positional-source attempt identified another omitted
required signature: `ReadAt::read_exact_at`. Source inspection confirms it is
required. The card now includes it, and row-encoding cards identify their `std`
feature requirement. Complete struct/enum cards are labeled "Type definition"
rather than "Trait methods". The existing source-extraction tests pass. This
check does not count as a fresh reader trial or a browser-layout measurement.
