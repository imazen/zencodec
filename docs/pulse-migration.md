# One operation control: Pulse

The review API carries progress, cancellation and phase planning through one
`Pulse`. Borrow it for a call; retain `SharedPulse` when a job outlives that call.
`NoPulse` is the static no-op control. It has no completion or cancellation state.

```rust
use zencodec::{NoPulse, Pulse, SharedPulse};

// Borrow through byte acquisition, codec steps, policy and destination work.
// decoder.step(input, sink, &NoPulse)?;

// Retain one owned operation control on a reusable registration context.
let codecs = zencodec::CodecSet::new().with_pulse(SharedPulse::default());
let pulse: &dyn Pulse = codecs.pulse();
pulse.check()?;
# Ok::<(), zencodec::enough::StopReason>(())
```

A progress-capable control must not be converted into `StopToken` internally.
That conversion retains cancellation but loses reporting and phase planning.
Typed jobs implement `with_pulse`; handwritten erased jobs implement `set_pulse`.
The provided erased adapters forward the complete `SharedPulse`.

A cancellation-only application can opt into the explicit adapter:

```rust
use zencodec::how_far::{SharedPulse, StopOnly};
let pulse = SharedPulse::new(StopOnly::new(zencodec::enough::Unstoppable));
let codecs = zencodec::CodecSet::new().with_pulse(pulse);
```

For an existing borrowed cancellation source use `StopOnly::borrowed(&stop)`.
Keep that adapter alive for the call. It cannot produce an owned shared view;
use `StopOnly::new(stop)` when the source can be owned. Do not substitute a no-op
when sharing fails.

## Migration surface

- Incremental decoding, controlled interpretation/resume, animation, policy
  resolution, row encoding, media send/receive and metadata services accept
  borrowed `Pulse` controls.
- Encode/decode jobs retain `SharedPulse`. `with_stop` / `set_stop` remain
  deprecated cancellation-only adapters; `with_pulse` / `set_pulse` are the
  required implementation methods.
- `CodecSet::pulse()` exposes its actual control. Legacy `with_stop` adds a
  cancellation condition without erasing the existing progress reporter.
- Opened collections retain a `SharedPulse` through their context and publication
  checks. The cancellation latch preserves the first observed reason. Child
  phases and shared views must retain that latch too.
- Bare byte `Request` becomes `InputRequest`; metadata-only `Request` becomes
  `RewriteRequest`. The old type names remain deprecated aliases.

This is a coordinated review-API migration, not a source-compatible 0.2 backport.
Native trait implementations pinned to older revisions need the signature and
owned-control updates before qualification. A codec that accepts a pulse does
not thereby report progress: report only completed work, with a truthful total
or an explicitly unknown total. Output byte timing, source passes and memory
retention remain separate contracts.

## Validation (2026-10-27)

The workspace passes all-feature tests, doctests and strict Clippy, the standalone
render tests, and no-default-features checks. The collection regression exercises
reporting, totals, issues, sharing, child phases and latched cancellation after
the collection is dropped. The API catalog and browser checks pass, including
source-exact copying, independent context expansion and graduated steps.

Commands: `just pulse-test`, `cargo test --manifest-path
zencodec-render/Cargo.toml --all-features --locked`, `cargo check --workspace
--no-default-features --locked`, `just code-catalog EXPORT`, `just api-doc`,
`just docs-site-browser-check`. Run from an isolated source export where needed.
Resource-wrapper receipts (build/test processes, not codec performance): combined
Rust gate rc=0, 18 s, peak RSS 0.42 GiB; catalog rc=0, 12 s, peak RSS 0.30 GiB;
browser gate rc=0, 29 s, peak RSS 0.20 GiB.

Native codec graph requalification and compile-time comparison remain required
before release. Pinned native adapters still implementing the old job methods
need migration; the workspace result does not qualify those external adapters.
