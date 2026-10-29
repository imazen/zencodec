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
