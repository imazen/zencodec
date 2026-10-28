# API usability, performance and impedance review (2026-10-27)

**Reading this against the current API:** this is a dated review, preserving
the original findings and proposals. The [current-head replay](#status-at-ideal-api-head-e8f16683-replayed-2026-10-28)
supersedes the old callback-bypass observations. It passes the four original
safety assertions; requirement scope and the recorded copy/allocation costs
remain open. Runnable controls are in the
[standalone probe crate](../perf/api-usability-validation/README.md).

An independent second-lane review of the checked-operation API on the review branch (base `6b0caf34`,
companion `zencodecs` pinned at zenpipe `77d316ba`, multi-pass prototype `b53cfe4e`). The review
started as a caller with only the guide sources and examples, then read the implementation.

Evidence labels:

- **source**: read in the named file and line, not executed;
- **probe**: produced by `perf/api-usability-validation` (count-based, no timings).

Probe results and provenance are in `perf/api-usability-validation/results_2026-10-27.tsv`: r7900x,
`run-heavy --mem 4G --jobs 2`, rc=0, build run 12 s with peak RSS 0.45 GiB, rerun 0.19 GiB.
Nothing here is a timing or memory bound. The probes count allocation calls and requested bytes
through dhat's heap counters, and they observe pointers inside a recording encoder.

Findings that other lanes already own are listed once at the end and not repeated.

## 1. What a first-time caller hit

These are the confusions recorded before reading any implementation, in the order they came up.

1. **The guide is an implementation log, not a guide.** `ideal-api.md` spends 178 lines on CMS
   qualification receipts before "The consumer shape". The checked page (`site/ideal.mjs`) is better,
   but it mixes executed scope, unqualified scope and receipts in every step.
2. **Five request types, three crates, six error types.** `DecodeRequest` / `EncodeRequest`
   (`zencodec::operation`), `TranscodeRequest` (`zencodecs::checked_transcode`), `CheckedCollection` +
   `OpenRequest` (core), `Renderer` / `RenderRequest` (`zencodec-render`), `row_encode::encode`
   (`zencodec-io`) and `PolicyDecoder` (`zencodec-media`). Their errors are `At<OperationError>`,
   `At<checked_transcode::Error>`, `At<CollectionOperationError>`, `At<RenderFailure>`,
   `At<row_encode::Failure>` and `media::policy::Error`. Each carries an `OperationReport` under a
   different accessor.
3. **"Policy" means four different things.** `EncodePolicy` / `DecodePolicy` are legacy carrier
   switches, `PolicySet` is the built-in resolver plus requirements, `PolicyResolver` is the callback,
   and `with_policy_defaults` is an adapter merge rule. `EncodeRequest` has `with_policy`,
   `with_policy_set` and `with_policy_defaults`, and `TranscodeRequest` has `with_encode_policy` and
   `with_policy_set`.
4. **Metadata is stated twice in the web example.** `with_metadata(DiscardDescriptive)` and
   `PolicySet::require_metadata(DiscardDescriptive)` appear together. Nothing tells the caller which
   one does the work. See finding A2.
5. **The guide's own method panel was incomplete.** "All implemented TranscodeRequest methods"
   omitted `transcode_with`, `transcode_from_read_at` and `transcode_from_read_at_with`, which the
   examples call. The capture regex required `name(`. *Fixed by the parent after this report,
   with regression checks.*
6. **The output-format decoder requirement is invisible.** The registry example registers
   `PngDecoderConfig` "to inspect finished output". A caller who only encodes PNG gets
   `CodecSetError::NoDecoder(Png)` at Prepare. That's correct and early, but the message points at the
   wrong mental model. See A6.
7. **Autocomplete leads to the best-effort path.** The value in hand is a `CodecSet`. Its ~30 methods
   (`decode`, `decode_preferring`, `decode_as`, `encode`, `encode_with`, `transcode`, `estimate_*`, …)
   are the legacy best-effort conveniences. The checked path is a separate type the caller has to
   already know about. `CodecSet::transcode` (core, best effort, no conversion) and
   `TranscodeRequest::transcode` (companion, checked) share a verb with different guarantees.
8. **Collections need a format up front and a magic number.**
   `open_collection(format, source, OpenRequest::new(view, discovery_attempts), resolver, stop)`
   doesn't detect, and nothing documents a sensible `discovery_attempts`. Still-image decoders and
   collection factories are separate registries (A9).
9. **A late-interpretation callback can't see what arrived late.** `Question::Boundary` and
   `Question::Transcode` give `InterpretationFacts { width, height, descriptor, state, unavailable }`.
   A callback can't tell that an ICC or EXIF/XMP carrier appeared, or that unknown carriers exist, so a
   content-dependent privacy rule ("refuse when unknown carriers are present") is impossible without
   payload access. See A7.
10. **The render region doesn't reduce work, and a raw layer can't be rendered.**
    `RenderRequest::with_region` crops after a full decode, and `ImageRequest::with_region` is a
    different (stored-coordinate) region. The docs say this, but the API shape invites the wrong
    assumption.

## 2. Caller attempts per scenario

| Scenario | What the caller writes today | Missing context / extra decisions |
|---|---|---|
| Upload to known sRGB, metadata excluded | `TranscodeRequest::new(Png).with_cms(&MoxCms).with_output(OutputProfile::Named(Cicp::SRGB), PixelFormat::Rgb8, ConvertOptions::retain_channels()).with_metadata(DiscardDescriptive)` | Must pick a CMS, a PixelFormat and a ConvertOptions preset (four presets, one deprecated). Must register the PNG decoder for verification. `allow_opaque_profiles(false)` is needed to *prove* profile exclusion, but it only filters; it never causes conversion. Whole-container privacy is unqualified and stated as such. |
| Required lossless | `EncodeRequest::new(f).with_fidelity(Fidelity::Lossless).encode(&codecs, px)` | Works and refuses honestly. `prefer_lossless(true)` is a preference with the same word. End-to-end lossless through a transcode also needs `OutputPlan::new_preserving_samples`, which `TranscodeRequest` doesn't expose. Its default uses deprecated `forbid_lossy()` (A5). |
| Slow network output, raw image not retained while blocked | `TranscodeRequest::transcode(..)` then write `output.data()` | Holds: the encoder is dropped before return, and the whole encoded output is retained. Live streaming with a pausable source exists only in `row_encode`, which takes a raw `Encoder` and bypasses `EncodeRequest` requirements (A8). |
| Seekable disk output | `row_encode::encode_to_destination(encoder, source, SeekBack::new(file)?, resolver, options, stop)` | Requires a hand-built `Encoder` from `codecs.encode_job(..)` (legacy, unchecked). There's no checked request → destination call. |
| Rendered selected TIFF/JXL vs raw layer | `codecs.open_collection(fmt, src, OpenRequest::new(Composited, N), resolver, stop)?` then `Renderer::new(storage).render_selected(&mut c, &id)` | Format must be detected by hand. The view is fixed per open, and the raw view also requires codec-specific factory configuration (`JxlCollectionConfig` + `Composition::Uncomposited`), so the same file needs two opens. The renderer refuses raw layers; placement handling is left to the caller. |
| Callback handling late interpretation | closure over `Question::Boundary` / `Question::Transcode` | The callback sees only dimensions/descriptor/state (item 9). Requirements set in the PolicySet are skipped when the callback chooses directly (A1). |

The feasible examples are in `perf/api-usability-validation` as count probes (section 4). The
native-dependency examples were not compiled in this lane: the parent owns that harness and the slot.

## 3. Prioritised API changes

### A1. Requirement composition: stated requirements don't survive a direct `Choose`

**Present behaviour, as documented.** `PolicySet` is described as the built-in policy a callback
"can use directly or delegate selected questions to with `Decision::UsePreset`". The code matches
that: `choose` (`src/operation/policy.rs:814-875`) builds `Choices` from the unfiltered offers and
accepts a callback's `Decision::Choose(id)` after only the ticket check. So this is not a deviation
from the docs. It's a composition footgun: a request built with `with_policy_set(strict)` reads as a
requirement, but a callback added later silently replaces it.

**Measured** (`perf/api-usability-validation/results_2026-10-27.tsv`, public `resolve_*` with
constructed offers and the strict preset `require_metadata(Discard)` + `allow_opaque_profiles(false)` +
`with_workspace_limit(1 GiB)`):

| Case | `UsePreset` | Direct `Choose(first)` |
|---|---|---|
| compliant offer | selected 0 | selected 0 |
| conflicting metadata (Preserve) | refused, `Declined` | **selected 0** |
| unknown evidence (opaque/workspace `None`) | refused, `Declined` | **selected 0** |
| noncompliant then compliant | selected **1** | **selected 0** |
| `EncodeRequest` (the domain has no evidence) | refused | **`Ok`** |
| Pending / Provisional boundary (deferred discovery) | `ContinueReading` | — |
| output schedule under `require_metadata` (unrelated domain) | refused | — |
| output schedule, wait limit 10 GiB, bound proven / unknown | selected / refused | — |

Every refusal displays `policy Declined at question N`. The record names no requirement.

**Proposed semantic strengthening: classify the fields first.**

| `PolicySet` setting | Class | Survives direct `Choose`? |
|---|---|---|
| `require_metadata(t)` | mandatory (privacy) | yes |
| `allow_opaque_profiles(false)` | mandatory (privacy) | yes |
| `with_workspace_limit(n)` | mandatory (resource) | yes |
| `with_output_wait_limit(n)` | mandatory (resource; the "10 GB streaming refusal") | yes |
| `require_output_schedule(s)` | mandatory (destination) | yes |
| `require_extended_range(true)` | mandatory (fidelity) | yes |
| `prefer_lossless(b)` | preference | no: ordering only |
| `allow_assumed_chroma_siting(b)` | situational permission: the default `false` is the preset's choice; an offer *discloses* the assumption, and a callback may knowingly accept it | no, unless a future `require_signaled_chroma_siting()` makes it mandatory |
| default `allow_opaque_profiles(true)` | no requirement | — |

A setter only records a mandatory requirement when the caller calls it. The admission mask is built
from mandatory fields alone, so the chroma-siting override and other assumption decisions stay
available to callbacks.

**Mechanism, no allocation and no cloned offers.** `MAX_OFFERS` is 64, so a private `u64` admission
mask is enough:

```rust
// policy.rs (private)
struct Admission { mask: u64, unmet: Unmet }
fn admit<T: Evidence>(offers: &[T], req: Mandatory) -> Admission;  // bit i = offer i admitted

// Choices keeps the original slice and indices; iteration skips unadmitted bits.
pub struct Choices<'a, T> { ticket: u64, entries: &'a [T], admitted: u64 }
impl<'a, T> Choices<'a, T> {
    pub fn iter(self) -> impl ExactSizeIterator<Item = Choice<'a, T>>; // len = admitted.count_ones()
    pub const fn len(self) -> usize;   // admitted count, not entries.len()
}
// choose(): Choose(id) on an unadmitted index -> DecisionCode::InvalidChoice with the
// violated requirements recorded; preferences then order only admitted entries.
```

`Choice::id()` keeps the original index, so a `Selection` and `preference_index` still identify the
prepared job without remapping. The question stays one variant per domain. An empty admitted set
still asks the callback with empty choices (as `Boundary` does now), so an application can return
its own traced error instead of the generic refusal.

**Diagnostics: name the requirement and the competing violations.** A data-free bitset fits the
payload-free `DecisionRecord`:

```rust
#[non_exhaustive] pub enum Requirement { Metadata, OpaqueProfile, Workspace, OutputWait,
                                        OutputSchedule, ExtendedRange }
#[derive(Clone, Copy)] pub struct RequirementSet(u16);
#[non_exhaustive]
pub struct Unmet {
    /// Requirements that excluded at least one offer.
    pub excluded_any: RequirementSet,
    /// Requirements every offer failed: the actionable cause when nothing remains.
    pub excluded_all: RequirementSet,
    /// Of those, exclusions caused by unknown evidence, not by a proven conflict.
    pub unknown_evidence: RequirementSet,
}
// DecisionRecord gains `pub unmet: Option<Unmet>`; Decline gains `Unmet(Unmet)`.
// Display: "required workspace bound unproven (2 of 2 routes); metadata conflict (1 of 2)".
```

For the measured `noncompliant_then_compliant` case, the record shows the selection plus
`excluded_any = {Metadata}`, so a reviewer can see that a route was withheld and why.

Core issue 173 tracks the mandatory-admission fix, which the parent owns. The present behaviour is
recorded as regressions in `perf/api-usability-validation/tests/policy_present.rs`. The proposed
semantics are in the opt-in `tests/policy_proposed.rs`, which fails against this base by design and
never replaces a present-behaviour assertion. Both files are kept unchanged as historical fixtures.
They compile only against base 6b0caf34, so they are built only with `--features
baseline-6b0caf34` (plus `proposed` for `policy_proposed`). At that base the 2026-10-27 run used
`--features proposed` before the baseline feature existed.

Run on r7900x at base 6b0caf34 through run-heavy (3G, 2 jobs): rc=0, 2 s, peak RSS 0.17 GiB.
`policy_present` passes 6 of 6. `policy_proposed` passes 1 of 4, as expected before issue 173:

| proposed test | result at this base | observed |
|---|---|---|
| `mandatory_requirements_survive_direct_choose` | fails | conflicting metadata: `Selected`, `used_preset: false` |
| `admission_mask_preserves_original_index` | fails | `Some(0)` selected, `Some(1)` expected |
| `refusal_names_the_requirement` | fails | text is `policy declined at question 4` |
| `preferences_do_not_become_requirements` | passes | `prefer_lossless` alone still lets the callback choose index 0 |

These are results for base 6b0caf34 only. They are not current-head regressions: the failed
`DecisionRecord` above has no `admitted` or `unmet_requirements` field because the base predates
mandatory admission.

#### Status at ideal-api head e8f16683 (replayed 2026-10-28)

Source identity: `e8f16683f90f2b2e6f4e7e1f1a474e61a3a301c4` (jj `wqqvwkrl`, bookmark `ideal-api`),
which contains `86223a80` (mandatory admission before callback selection) and `5f10fcb2` (the
test-only policy binding experiment). Read: `src/operation/policy.rs` (`choose` at 870-977,
`PolicyFailure` Display at 781-791), `src/operation/policy/requirements.rs`, `src/operation.rs`
(`encode_with` 594-635, `decode` 263-300), `zencodec-io/src/row_encode.rs`. Head `choose` builds a
`u64` admission mask over original indices, refuses before the callback when nothing is admitted,
and records `offered`, `admitted` and `unmet_requirements` (a union of `(PolicyRequirement,
RequirementEvidence)` pairs).

The baseline tests can't compile at head as written. Every `resolve_*` and `row_encode::encode` now
takes `&dyn how_far::Pulse`, which `enough::Unstoppable` does not implement. The head tests change
only that argument (`static STOP: &zencodec::how_far::Inert = &zencodec::NoPulse`). In the landed
crate the head tests (`head_present`, `head_proposed`) and the Pulse stop argument are the defaults,
so `cargo test` and `cargo run` in `perf/api-usability-validation` target the current core.
`dependencies.lock` is the tracked graph: copy it to the ignored `Cargo.lock` and build with
`--locked`. It is the run02 lock, which differs from the uncommitted baseline lock only in
zencodec 0.1.27 → 0.2.0.

| Baseline row (6b0caf34) | Applies at head? | Observed at head (replay) |
|---|---|---|
| `UsePreset`: compliant → 0; noncompliant then compliant → 1 | yes, unchanged | same |
| `UsePreset` refusals have code `Declined` | yes, unchanged | same |
| `UsePreset` refusals have `used_preset == true` | superseded | `false`: refusal precedes any callback, so nothing delegates |
| direct `Choose` selects a conflicting or unknown-evidence route | superseded (issue 173) | refused before the callback; `admitted == 0` |
| direct `Choose(first)` on `[conflicting, compliant]` selects 0 | superseded | selects original index 1; `offered 2`, `admitted 1`, unmet `Metadata(Contradicted)` |
| strict `EncodeRequest` refuses; with a choosing callback, `Ok` | superseded in part | both refuse before the callback with unmet `Metadata`, `OpaqueProfiles`, `Workspace` (all `Unknown`) |
| Pending/Provisional boundaries continue | yes, unchanged | `ContinueReading` is exempt (`requirements.rs`, Boundary arm) |
| output schedule under `require_metadata` refuses | yes, unchanged | refuses with unmet `Metadata(Unknown)` |
| wait limit: proven passes, unknown refuses | yes, unchanged | same |
| refusal text names no requirement | superseded | `policy Declined at question N: unmet requirements OpaqueProfiles(Unknown) Workspace(Unknown)` |
| all four `policy_proposed` expectations | apply | pass |

What still applies from this review at head:

- **A1 diagnostics, in part.** The text names requirements only when nothing was admitted
  (`admitted == 0`). A selection that withheld routes carries the union in the record but not in any
  text. The union does not separate "every route failed X" from "some route failed X". The refusal's
  `source()` is still whatever `Decline` the preset returns for the unfiltered question
  (`policy.rs:922-937`), not a requirement-specific cause. The parent tracks this as the open two-placement diagnostic gap; it's
  listed here, not re-reported.
- **A1 empty admitted set.** Head refuses without asking the callback. This review proposed asking
  with empty choices so an application can return its own traced error. Head's order is fail-closed
  and simpler; this is a design choice, not a defect.
- **A2 holds unchanged.** The only scope distinction at head is the exemption for discovery
  `ContinueReading`. A transcode-shaped tenant preset still refuses a plain `EncodeRequest` and an
  output-schedule question, now with `Unknown` evidence instead of `UnsupportedQuestion`. Evidence
  is still per-question `Option` fields, with no `NotApplicable`/`Deferred` scope.
- **Unchanged structure (source only, not measured at head):** P2 required descriptor compared after
  `decode()` (`operation.rs:289-295`); P3 `Raster::read_rows` copies each row
  (`row_encode.rs:139`); P4 `stage` allocates one `OwnedStorage` per non-empty chunk
  (`row_encode.rs:386-397`).
- **A8, in part.** `row_encode::encode_dyn_job_to_destination` (from `f627b02b`) runs an erased
  registry job to a `Destination`. There is still no `EncodeRequest`-level entry, so request
  requirements are not carried into that path.

Replay, separate from the baseline: `perf/api-usability-validation/head-replay.sh <new-run-dir>
<rev>`. It records the resolved revision and a hash of the crate files, exports the revision with
`git archive` and copies this crate in, with `dependencies.lock` as `Cargo.lock`. It then checks
that `head_proposed.rs` equals `policy_proposed.rs` apart from the `STOP` line. Finally it runs
`head_present`, `head_proposed` and the probe binary (`--locked`) and checks that the lock is
unchanged. `head_present.rs` repeats the still-applicable baseline assertions unchanged. For each
superseded row it states head semantics at least as strictly: the callback panics if it is called,
and the record's counts and exact unmet pairs are checked.

The runner fails closed:

- It refuses an existing run directory, so it never replaces or deletes an earlier run.
- The export pipeline uses `pipefail`.
- Every external command is bounded by `REPLAY_STEP_TIMEOUT` (default 900 s).
- Each step's full output goes to its own log, and `steps.tsv` records every exit status.
- Any failed step makes the runner exit nonzero.

`head-replay-controls.sh` proves this with fake `git` and `cargo` on `PATH`, without a build. It
runs one positive control and nine failure controls:

- failing tests;
- failing probes;
- a failing export;
- an export that fails after writing valid output;
- an unresolvable revision;
- a hung step;
- a changed lock;
- an occupied run directory;
- assertion drift between the baseline and replayed proposed tests.

The new runner passes 22 of 22 checks. Pointed at the run02 runner (`REPLAY_SCRIPT`, revision
`e00f9d7c`), the controls catch six false successes: failing tests, failing probes, the export
failing after output, the hung step, the changed lock, and the occupied directory (whose sentinel
was replaced). The drift control is not valid against that runner, because the edit lands in a
different copy.

**Replay result** (r7900x, 2026-10-28, `run-heavy --mem 3G --jobs 2`, rc=0, 12 s, peak RSS
0.45 GiB). The source is an export of `e8f16683f90f2b2e6f4e7e1f1a474e61a3a301c4`. The current
`ideal-api@origin`, `d420712a`, changes only docs and `perf/jpeg-policy-validation` since then, so
the same core code was tested. The probe crate is `e6760e42`, and the only lock change was zencodec
0.1.27 → 0.2.0. This run used the earlier runner (feature `head`, no exit aggregation), so
the per-step logs, not the runner's exit status, are its evidence. All four steps returned 0 in
`steps.tsv`.

- The check that the replayed proposed tests equal the baseline ones passed.
- `head_present` passed 9 of 9 and `head_proposed` 4 of 4. Every row in the table above was
  observed, not only predicted.
- The probe output is in `perf/api-usability-validation/results_head_e8f16683_2026-10-28.tsv`.
- Unchanged from the baseline:
  - P2: a refused required descriptor allocated 786,880 B.
  - P3: 0 of 64 pushes borrowed the caller's raster.
  - P4: the 8-object pool was refused with `Objects { limit: 8 }` after 8 pushes.
- `start_controlled` requested 24 B fewer per call than at the baseline.
- Each tenant-preset encode refusal names its one requirement, for example
  `policy Declined at question 4: unmet requirements Metadata(Unknown)`.
- An output wait with no known bound reports `OutputWait(Unknown)`.

The baseline results for 6b0caf34 above are kept unchanged; the replay did not rerun them.

**Tests the canonical implementation should carry** (the probe matrix is the template): direct
`Choose` of an admitted and an unadmitted offer; `UsePreset`; conflicting metadata; unknown evidence
for each mandatory field; deferred discovery (Pending/Provisional boundaries continue under every
mandatory requirement); an unrelated operation (under A2 an output-schedule question with a metadata
requirement is `Deferred` or `NotApplicable`, not a refusal); assumed chroma siting accepted by a
callback while the preset declines it; and original-index identity of the chosen job after masking.

### A2. State requirements once on the operation: evidence scopes and final-effect discharge

The parent's constraint: keep one resolver and one configurable built-in object, so a tenant can
state "refuse when route retention exceeds 10 GB" once and reuse it. The current problem isn't the
single object. The preset can't tell an irrelevant domain from an unproven final effect, so it
declines both (`Decline::UnsupportedQuestion`). A reusable tenant preset therefore breaks
`EncodeRequest`, `CheckedCollection`, `Renderer` and `row_encode` at runtime, and the refusal names
no requirement.

Proposal: keep `PolicySet` and its builder methods unchanged. Give every offer the same evidence
block, and let the operation's adapter declare its role:

```rust
#[non_exhaustive]
pub enum Scope<T> {
    /// The executing route establishes this value.
    Proven(T),
    /// This operation performs no such effect (adapter-declared, e.g. in-memory
    /// pixel publication emits no destination metadata).
    NotApplicable,
    /// A later question in this same operation will carry the evidence.
    Deferred,
    /// Unknown. Never satisfies a requirement.
    Unknown,
}
#[non_exhaustive]
pub struct Evidence {
    pub metadata: Scope<MetadataTransfer>,
    pub opaque_profile_retained: Scope<bool>,
    pub workspace_upper_bound: Scope<u64>,
    pub wait_retained_upper_bound: Scope<u64>,
    pub schedule: Scope<OutputSchedule>,
    pub extended_range: Scope<bool>,
}
// Each offer type gains `pub evidence: Evidence`, replacing today's ad-hoc
// Option fields (retains_opaque_profile, workspace_upper_bound,
// wait_retained_upper_bound, preserves_extended_range).
pub enum Effect { Intermediate, Final }   // passed to resolve_* by the adapter
```

Rules in `choose` (shared, fail-closed):

- `Proven(v)` is checked against the requirement.
- `NotApplicable` discharges the requirement. Only an adapter whose operation has no such effect may
  claim it, and the claim is recorded in the `DecisionRecord`.
- `Deferred` is allowed only at `Effect::Intermediate`. The requirement stays pending in the
  operation's ledger, which is kept with the `OperationReport` that every operation already carries.
- At `Effect::Final`, `Deferred` and `Unknown` refuse with `Decline::Unmet(Requirement::X)`, which
  names the requirement.

The question carries `pending: RequirementSet` (data-free) so the single callback can see what is
still undischarged. Example: the 10 GB rule is `PolicySet::new().with_output_wait_limit(10 << 30)`.
`TranscodeRequest` today offers `wait_retained_upper_bound: Unknown` (the encoded output is retained
in memory), so it refuses with "output-wait bound unproven", not "unsupported question". A
`CheckedCollection` image publication declares `NotApplicable` for destination waits and passes. A
`row_encode` FinishFirst offer with memory staging can state `Proven(storage.max_bytes())`.

This touches the offer structs (non-exhaustive, review-only), `choose` and the `resolve_*`
signatures. The `PolicySet` builder surface doesn't change.

**Metadata duplication, exact simplification.** With A1 in place, `PolicySet::require_metadata` is
enforced on callback choices too, so the request field can derive from it:

```rust
// zencodecs/src/checked_transcode.rs
- metadata: MetadataTransfer,
- pub fn with_metadata(mut self, policy: MetadataTransfer) -> Self { .. }
+ /// Sugar: equivalent to with_policy_set(self.preset.require_metadata(policy)).
+ pub fn with_metadata(mut self, policy: MetadataTransfer) -> Self {
+     self.preset = self.preset.require_metadata(policy); self
+ }
  // execution.rs: the prepared treatment is read from the one stored value
- self.request.metadata
+ self.request.preset.metadata_requirement().unwrap_or(MetadataTransfer::PreserveReported)
```

`with_policy_set(p)` after `with_metadata(x)` must not silently drop `x`. Make `with_policy_set`
refuse a conflicting value at execution (`Refusal::ConflictingMetadataRequirement`), or merge only
`None` fields, the same rule `with_policy_defaults` already uses. This needs one `pub(crate)`
or `#[doc(hidden)]` getter on `PolicySet`, or the transcode adapter moves into core. A reusable tenant
preset stays a single `PolicySet` value, and fail-closed holds because the prepared route and the
requirement come from the same stored value. **Do not delete `with_metadata` before A1 lands**: today
it is the only form that survives a callback that chooses directly.

### A3. Give refusals the requirement that failed

`Decline` is `ApplicationRule | NoPreferredChoice | UnsupportedQuestion`. A preset refusal for an
unproven workspace bound and one for metadata look identical to the caller. Add
`Decline::Unmet(Requirement)`, where `Requirement` is a data-free `#[non_exhaustive]` enum. The record
stays payload-free.

### A4. One checked entry point per verb; demote the legacy conveniences

`CodecSet::decode/encode/transcode` should be the checked operations (`DecodeRequest::new().decode`
etc.) in 0.3, or be marked `#[deprecated(note = "best effort; use operation::DecodeRequest")]`
once the checked path covers their consumers. Rename the core best-effort `CodecSet::transcode` to
`transcode_best_effort` in the breaking release so it can't be confused with the companion
`TranscodeRequest::transcode`.

### A5. Transcode defaults must not use the deprecated preset

**source** zenpipe `77d316ba` `zencodecs/src/checked_transcode/execution.rs` ~L140: without
`with_output`, the route uses `ConvertOptions::forbid_lossy()`. The default is meant to be "preserve
current color", so it should use `OutputPlan::new_preserving_samples`. That's the exactness contract
the guide introduced for this case, and it never consults a CMS.

### A6. Name the verification-decoder requirement

`TranscodeRequest` verifies output by decoding it and fails Prepare with `NoDecoder(target)` when no
decoder for the target is registered. Add `Refusal::VerificationDecoderMissing(ImageFormat)` and one
doc line on `TranscodeRequest::new`. The check already happens before work; only the message is
misleading.

### A7. Data-free carrier facts at interpretation boundaries

```rust
#[non_exhaustive]
pub struct CarrierFacts { pub icc: Presence, pub exif: Presence, pub xmp: Presence,
                          pub unknown_carriers: u16, pub complete: bool }
pub enum Presence { Absent, Present, NotYetKnown }
// InterpretationFacts gains `pub carriers: CarrierFacts` (non-exhaustive struct: additive).
```

This lets one callback refuse unknown carriers or late EXIF without exposing payloads. `complete`
distinguishes "absent" from "not inspected", the same distinction the native JPEG qualification
found missing in `probe_full`.

### A8. A checked request → destination path

```rust
// zencodec-io (std)
pub fn encode_request<S: row_encode::Source, D: Destination, R: PolicyResolver>(
    request: &EncodeRequest, codecs: &CodecSet, source: S, destination: D,
    options: row_encode::Options, resolver: R, stop: &dyn Stop,
) -> Result<Outcome<row_encode::Completed>, At<row_encode::Failure>>;
```

This would prepare jobs through `request.prepare(codecs)`, ask one `Question::Encode`, then the
existing `OutputSchedule` question, or one joint question under A2. `TranscodeRequest` gets the
matching `transcode_to(.., destination)`. Then "slow network output" and "seekable disk output" are
the same checked request as the in-memory one.

### A9. Collections: one registry, detection, a default

- A blanket single-image collection over each registered still decoder, so `open_collection` works for
  every registered format and codec-agnostic callers write one path.
- `open_collection(source, OpenRequest)` detects from a positional prefix read
  (`ImageFormat::RECOMMENDED_PROBE_BYTES` via `read_at`). That isn't source staging.
  `OpenRequest::with_format(f)` overrides detection.
- `OpenRequest::composited()` / `::layers()` with a documented default discovery allowance. Keep
  `new(view, attempts)`.
- A named alias: `pub type OpenedCollection<'a, R> = CheckedCollection<Discovery<ErasedCollection<'a>>, R>`.

### A10. Semver hygiene on the new render types

`RenderRequest` and `Renderer` have public fields without `#[non_exhaustive]`
(`zencodec-render/src/render.rs:30,151`). Adding a field later breaks callers. Make them
non-exhaustive with builders, or make the fields private.

### A11. One source-access enum before multi-pass lands

`row_encode::Source::mechanism() -> OutputSource { BorrowedRaster, PullRows, Unknown }` and the
prototype `Replay { Borrowed, Positional, Forward }` describe the same property. Merge them into
`SourceAccess { Borrowed, Positional, Forward }` on the one `Source` trait. A single-pass encoder is
a multi-pass encoder with `PassCount::Exact(1)`, so one pull driver (`encode_passes`) can host both.
Implement `row_encode::encode` as that driver plus a push-encoder adapter, rather than keeping two
row loops. Replay then becomes evidence on the route (`Scope<u32>` source passes) under A2, not a
second resolver trait (`ReplayResolver` / `ReplayDecision` / `ReplayPolicy` in the prototype).

## 4. Performance review

| # | Cost | Where | Evidence | Smallest discriminating check |
|---|---|---|---|---|
| P1 | Per-operation clone of the registry: two `Vec`s plus a `clone_entry` per registered config | `src/set.rs:1041`; `DecodeRequest::start_controlled` (`src/operation.rs:254`) and `CodecSetIncrementalDecoder` hold an owned set | **probe**: `CodecSet::clone` = 2 allocations, 32 B per decoder+encoder pair (0/1/4/16 pairs → 0/32/128/512 B); `start_controlled` = 4 allocations. This is a toy measurement and **not** a real-config clone cost: the testkit configs are zero-sized, so their `Box` clones don't allocate. For real configs with fields, one allocation plus a deep config copy per entry is source-inferred and not measured | Fix: `CodecSet { registry: Arc<Registry>, stop, pulse }` (clone = 1 refcount). Measure with the native JPEG/PNG configs in the parent's harness |
| P2 | Required output descriptor refused only after a full decode and raster allocation | `src/operation.rs:287-298` | **probe**: 512×512 RGB8 source, required BGRA8: the refused call allocated 786,880 B (raster 786,432 B) in 6 allocations; the matching request allocated 786,616 B | Fix: compare the negotiated descriptor before `decode()` |
| P3 | Borrowed raster copied row by row into the batch buffer before `push_rows` | `zencodec-io/src/row_encode.rs` `Raster::read_rows` | **probe**: 0 of 64 pushed batches point into the caller's raster | Fix: optional `Source::borrow_rows(first, n) -> Option<PixelSlice>` |
| P4 | Finish-first memory staging: one `OwnedStorage` allocation plus copy per non-empty pending chunk; each counts against the object cap | `row_encode.rs` `stage()` | **probe**: 256×64 RGB8, 1-row batches, 1 B emitted per push. With cap 4096: 64 pushes, 80 allocations, ok. With cap 8: refused at Encode with `Objects { limit: 8 }` after 8 pushes, i.e. 8 bytes of output exhaust an 8-object pool | Fix: one growable admitted buffer, or chunk coalescing |
| P5 | Transcode verification parses the encoded output twice (`probe_full` + `output_info`) | zenpipe `execution.rs` `finish` | source | Count decoder parse calls with a counting decoder; merge into one inspection |
| P6 | Renderer: projection always copies, then conversion copies again; a full poison fill (`filled(.., 0xa5a5…)`) precedes conversion; conversion runs one row per `convert_rows` call; stride `((w+8)&!7)*bpp` always pads 1–8 px (`(w+7)&!7` was probably intended) | `zencodec-render/src/render.rs` `apply`/`convert` | source | Bytes allocated per default render vs 2× output size. Fix: borrow projection at identity orientation/full region; convert batches of rows; poison only under `debug_assertions` |
| P7 | Render region crops after the full native decode | `render_selected` passes `ImageRequest::new()` | source, documented | Map the display region through inverse orientation into `ImageRequest::with_region` where the adapter supports it. No contract change |
| P8 | `row_encode::validate_source` compares `Description` three times per batch, including `Option<Arc<ColorContext>>` | `row_encode.rs` | source; checked: `ColorContext` derives `PartialEq`, and std's `Arc<T: Eq>` equality short-circuits on pointer identity, so this is not O(ICC length) | none needed |

Not measured: wall time, RSS, native workspace. Speed benchmarks would need parent coordination and
zenbench on an allowed host (not dev).

## 5. Impedance table

| Codec / interface | Natural native shape | Checked API today | Core responsibility | Companion (`zencodec-io` / render / media) | Native implementation |
|---|---|---|---|---|---|
| JPEG decode | Forward parse; ICC in APP2 before scan; EXIF/XMP usually early; MPF/gain map after EOI | Whole-source controlled route qualified; `probe_full` defaults to the header probe (known bug) | Boundary facts incl. carrier completeness (A7) | Driver keeps input; transcode route | Byte-resuming reconstruction; complete-carrier claim |
| JPEG encode | Baseline streams rows; progressive retains all coefficients | `EncodeRequest` + `TranscodeRequest`; output returned whole | `OutputTiming` per configuration | `encode_request` to destination (A8) | Report `streams()` honestly per mode |
| PNG decode | Rows/Adam7; `iCCP` before IDAT; `eXIf`/text may follow IDAT (late) | Qualified incl. late EXIF; APNG/gain map refuse | Same | Same | Same |
| PNG encode | Streams rows through deflate; Forward output fine | Same as JPEG | — | A8 | — |
| TIFF read | Random access by offset; IFD graph with shared SubIFDs; external JPEG tables | `ReadAtReader` + private collection adapter; packaging open | Collection identity/region contract (done); detection (A9) | Source-version binding (HTTP/clone sources qualified privately) | Packaging; native region decode for the render region (P7) |
| TIFF write | Offsets need seek-back or full buffering | Not exposed through checked requests | `OutputAccess::SeekBack` exists | `encode_request` + `SeekBack` (A8) | Patch-at-finish layout |
| JXL decode | Frames/layers, presentation needs earlier frames; container boxes (EXIF/XMP/jbrd) may follow the codestream | Packaged collection; view fixed per open | View per open is right; a second open for raw is a cost, not a contract issue | Renderer refuses raw layers (correct) | Random raw seeking (open) |
| JXL encode | Whole-image global passes typical | `EncodeRequest` only | — | A8 | Report pass count once multi-pass lands |
| IMI multi-pass encode | Needs ≥2 reads of each row; bounded seek-back output | Prototype `encode_passes` + separate `ReplayResolver` | One `Source` with `SourceAccess`; replay as route evidence (A11, A2) | One pull driver | `begin_passes`/`pull_rows` |
| AV1/MP4 video | Packet/frame sessions; moov at end unless fast-start; muxer patches | `PolicyDecoder`, staged encode, private `PreparedOutput` | `Question::Frame`/`FrameRoute` (done) | Public facade owning preparation (open) | Plane admission (done); workspace bounds (open) |

### zenpixels items this review needs

None of the A-changes require a zenpixels change. P6 benefits from one existing-shape addition, a
batched `OutputPlan::convert_rows` over multi-row strided views. Whether multi-row input already works
needs a check against the pinned revision; the renderer currently passes one row per call.
Finalization ownership remains zenpixels issue 69, as already recorded.

## 6. Release-contract cut

Contract blockers (these change signatures or semantics; land them before 0.3):

1. A1: requirement enforcement on callback choices. Today it is fail-open.
2. A2: evidence scopes and final-effect discharge on offers, plus `Decline::Unmet`. This reshapes every
   offer struct and the `resolve_*` signatures.
3. A7: carrier facts on `InterpretationFacts`. Late-metadata privacy depends on it.
4. A8: a checked request → destination path. Without it, streaming and seek-back output stay outside
   the checked contract.
5. A11: one source trait and one access enum before multi-pass is public.
6. A10: non-exhaustive render types, before `zencodec-render` publishes.
7. A4: decide which `CodecSet` verbs are checked in 0.3. The return types differ, so this is
   breaking.

Implementation details that can't change contracts: P1–P8, A5, A6, A9's detection and defaults
(additive), and native region decode for render.

## 7. Owned elsewhere (not re-reported)

- Guide capture regex dropped generic methods: found here, fixed by the parent.
- Preset final-`Boundary` bypass before `PolicyDecoder` sink output: parent.
- Multi-pass prototype issues the parent already requested from the first lane: swallowed row-fill
  errors on Done, no-progress/multiple callbacks, early completion, zero pass budget, digest vs
  immutable-source evidence.
