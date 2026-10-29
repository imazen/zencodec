# Maintaining the API guide

`ideal.html` is the current checked API guide. `index.html` introduces the three
roles and earlier experiments; `migration.html` remains unlisted. All source
lives in `docs/`; GitHub Actions builds, checks and deploys the main revision.

When a trait or adapter changes, update its example, extracted signatures and
the relevant entry in `improvements.mjs` in the same change. Each entry states
the problem, the consequences for all three roles, evidence and remaining
limits. Keep released, implemented-on-PR and proposed behavior distinct.

Signatures are selected by name in `api.mjs` and read from Rust source. Complete
examples live in `docs/examples/`; the native harness imports those exact files.
Cross-repository methods come from `integration-api.json`, whose revision must
match the pinned, tested integration. Do not replace these with illustrative
signatures that cannot compile.

Run the relevant native example gate and `just docs-site-browser-check 3547`.
The browser checks above/below expansion, whole-example copying, keyboard use
and horizontal overflow at three viewport sizes. Keep rationale brief in the
collapsed view; place details and evidence in the expansion. Do not turn a
successful narrow test into a claim that the complete target is implemented.

`gaps.html` lists existing API names against missing behavior and qualification.
Its canvas section distinguishes the implemented public rendering companion
from remaining default rendering and composition work; the table must not imply
that unprocessed packed or composited output is display-ready. Compile observations retain
Zenbench resource flags and distinguish the traits consumer from codec builds.

## Code-first progression qualified on 2026-10-28

Each role now opens with basic usage. The consumer starts with registration,
decode and transcode, with its dependency manifest linked beside the examples.
Setup above and continuation below expand independently. The cancellation
recipe accepts the application's `SharedPulse`; the file recipe keeps the
operation policy and checks cancellation before opening the destination.

Public borrowed controls now take `Option<&dyn Pulse>`. Beginner driver examples
use `None` for no additional per-call control. Keep any set/job `SharedPulse`
active, and forward the original option through adapters. `Some(pulse)` carries
reporting, phase planning and sharing along with cancellation. Required policy
and resource checks still apply. The default policy selects a permitted route;
later evidence can require a scoped callback before affected output is released.
See the [optional control contract](../optional-pulse.md).

The [native receipt](../../benchmarks/code_guide_2026-10-28.json) records the
five new cancellation/publication controls, full JPEG harness, preflight, lint
and browser checks. Captured integration signatures must match both the native
dependency revision and the `OperationPolicy` accepted by the copied examples.
Synchronous file writes are not transactional or preemptively cancellable.
That receipt predates optional borrowed controls; it does not qualify the new
signatures or their native adapters. Re-run the matching native guide, source
catalog and browser gates after integration.

## Source-backed codec-detail methods (2026-10-28)

The compiler catalog omits the three inherent `codec_details` methods on
`dyn SourceEncodingDetails`, with `Send`, and with `Send + Sync`. The searchable
site now adds their declarations and documentation directly from `src/detect.rs`.
The original compiler files and counts remain unchanged. Source drift, lost public
visibility, additional methods, or compiler coverage of these methods fails the
build so the explicit supplement can be reviewed.

`just source-details-check` executes real consumer calls for all three forms,
including identity-preserving downcasts and mismatched-type refusal. Site and
browser checks retain the existing catalog coverage and add search/source-link
checks for the supplement. [Validation receipt](../../benchmarks/source_catalog_2026-10-28.json).

## Qualified integration snapshot (2026-10-29)

The JPEG harness uses zenpipe `f0322466`. Recapture its named declarations
with `just checked-guide-api <zenpipe-checkout> <full-revision>` whenever that
dependency changes. The generated revision must match the native manifest,
even when the declarations themselves are unchanged.

On `9b606366`, all 48 native JPEG/conversion scenarios, strict harness Clippy,
24 source tests and the complete browser suite pass. The lock delta changes
only zencodecs' Git revision. The paired consumer separately qualifies its full
HEIC/RAW feature-profile compilation and API snapshot generation; that is not
whole-codec/platform runtime coverage. Two AVIF encode-orientation carrier
assertions still conflict with native baking and remain unresolved.
See [the receipt](../../benchmarks/guide_integration_2026-10-29.json).

## Deterministic method ownership (2026-10-29)

Rustdoc lists some implementations under multiple involved types. The source
catalog must attach a method to the implementation's resolved `for` type,
not whichever type happens to appear first in the JSON index. Generic blanket
methods remain documented at their trait declaration; they must not acquire
an arbitrary concrete owner. Compiler-generated signature files are unchanged.

`docs/test_code_catalog.py` permutes an index containing shared conversion and
blanket implementations. It failed before the ownership filter and passes after
it; the documentation workflow and `just docs-site-check` retain this control.
Regenerate all eight catalogs after changes to crate source, manifests or
READMEs. Rebuild Rustdoc and the guide from the same revision before browser
checks; a stale reference is deliberately refused.
