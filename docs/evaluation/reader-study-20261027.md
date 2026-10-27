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
