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
Its canvas-rendering recommendation remains proposed; the table must not imply
that packed or composited output is display-ready. Compile observations retain
Zenbench resource flags and distinguish the traits consumer from codec builds.
