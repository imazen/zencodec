# Video admission before native work

```rust,ignore
let decoder = PolicyDecoder::try_new(
    Av1VideoDecoder::new,
    PolicySet::new(),
    None,
)?;
```

Default policy permits construction and checks each presentation before returning
its frame. Add a required workspace bound only when refusal is acceptable:

```rust,ignore
let decoder = PolicyDecoder::try_new(
    Av1VideoDecoder::new,
    PolicySet::new().with_workspace_limit(64 << 20),
    None,
)?;
```

This generic adapter has no qualified native workspace evidence. The second
example therefore refuses with a typed `UnmetRequirements` naming Workspace /
Unknown, before calling the factory. Even `u64::MAX` does not turn unknown into
proof. A callback cannot waive bound requirements. This provides predictable
refusal, not a successful bounded native decoder implementation.

Keep native preparation inside the factory. Capturing an already-created decoder
cannot undo its allocations. The existing `PolicyDecoder::new` remains useful for
wrapping an existing session; its policy governs subsequent operations only.
Both paths check bound and legacy workspace requirements before configure, send,
receive, end_input and reset. Refusal drops the retained native owner and is
terminal. No fabricated choice or callback decision is recorded for this early
failure; its named cause remains available through the error chain. Cancellation
is checked first without an extra poll in the adapter.

Other late frame/metadata decisions remain at publication. This workspace check
does not certify metadata completeness, opaque-data privacy or native references'
memory cost. Supporting success under a strict bound still needs trustworthy
native preparation, reference-storage and conversion accounting for the same
retained owner.

## Qualification

The before-fix regression at95caa13d fails because native configure succeeds.
The repaired controls exercise all five entry points, legacy and bound policies,
zero native/callback calls on refusal, owner cleanup, terminal state, named
unknown evidence, default factory success, factory errors and cancellation.
The late-refusal optional-Pulse control retains its assertions using descriptive
metadata; workspace refusal now has its separate earlier-effect matrix.

On edb9f787, the full media testkit and strict all-target Clippy pass. Two native
AV1 controls pass: strict unknown workspace never invokes the rav1d constructor;
default admission constructs/configures the real decoder and preserves a typed
native failure. The existing erased native error/trace control also passes.
Receipt: rc0,18s,peak RSS0.97GiB,minimum available10468MiB,peak load0.80.
This is validation resource usage, not a decoder workspace measurement.

Commands: `just media-workspace-admission-check` and
`just media-native-workspace-check`. Logs and exact before/after source exports:
`/home/lilith/tmp/zencodec-video-admission-20261029/`. The initial missing-lock
attempts and missing-std test-runner failure remain retained. No test was skipped
or behavioral threshold weakened.

API snapshots and the eight-crate compiler-derived catalog regenerate and pass
freshness checks. The migrated native guide example compiles. All24 source/site
checks and the complete browser suite pass (rc0,42s,peak RSS0.33GiB); all eight
Rustdoc crates build with warnings denied (rc0,6s,peak RSS0.32GiB). The first
site attempt lacked Node dependencies and built HTML; those prerequisites were
installed/built before rerunning the unchanged checks.

### Cancellation during construction

The factory boundary also polls after successful native construction. If that
poll stops, the unpublished decoder drops before return; a failed constructor
still preserves its original error. Long constructors must poll internally—an
outer boundary cannot preempt synchronous work. A regression against5c4b4a77
with the corrected borrowed-control fixture returns a decoder after timeout;
the fixed control requires the exact TimedOut cause and zero retained storage.

The corrected borrowed-control witness fails before the fix with “published a
decoder after construction cancelled”. The full media testkit, strict Clippy
and catalog regeneration pass onb9cc4431: rc0,13s,peak RSS0.29GiB,minimum
available11115MiB,peak load0.33. The initial fixture lifetime compile failure
and negative-control failure are retained under
`/home/lilith/tmp/zencodec-video-cancel-20261029/`; no old assertions changed.

## Staged encoding preparation

StagedEncode now checks bound workspace requirements before reading/cloning
the encoder configuration, building tracks or preparing the muxer. Its generic
route has no qualified native/mux bound, so an explicit cap—including u64::MAX—
refuses with Workspace/Unknown at Stage::Policy. Default policies still prepare
and run normally. This does not cover construction of encoder, mux or staging
objects supplied by the caller; they already exist when this constructor runs.
Late frame decisions cannot retroactively authorize preparation allocations.

The corrected no-mux-call witness fails against2fbf58ca, then passes with
the early gate. On7ca0e816, the full media testkit and strict Clippy pass;
actual AV1/MP4 strict refusal releases the preconstructed encoder with no writes.
The unchanged default staged pipeline passes independent FFmpeg/libdav1d pixel
comparison and writer-failure controls; existing conversion-drift, unwind, epoch,
fidelity, cancellation and budget refusals also pass. Catalogs regenerate.
Receipt: rc0,17s,peak RSS0.40GiB,minimum available11192MiB,peak load0.44.
Command: `just media-staged-admission-check <fresh-artifacts-directory>`.
The retained oracle output is `/mnt/v/output/zencodec/staged-admission-20261029/`;
logs/source exports are `/home/lilith/tmp/zencodec-staged-admission-20261029/`.
The initial test-wrapper missing take_output compile failure is retained.
The original empty-operation requirements matrix keeps every refusal/no-output
assertion and now accepts refusal during construction as well as finish.
