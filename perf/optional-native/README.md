# Optional Pulse native qualification

`prepare.py REPOSITORY REVISION DESTINATION CORE_EXPORT` exports a frozen native
commit outside the parent workspace overrides. It preserves the initial lock and
records the exact manifest bridge. Only core-family paths and paired pixel pins
change. It refuses to overwrite an export. Run Cargo resolution under the shared
resource wrapper, review its lock delta, then use locked tests and strict Clippy.

This is an integration experiment. An export-only bridge does not establish that
a portable native manifest resolves; the final pushed graph needs separate
qualification. Keep the original failure logs and source identity when repairing
fixtures or orchestration. Existing pixel, cancellation and privacy assertions
remain binding. No binary archives are retained by this helper.

For portable landing graphs use `audit-lock.py EXPORT --portable`. This rejects
an additional legacy core, duplicate family packages, mismatched core-family
sources and different pixel/converter sources. The older experiment mode permits
an isolated legacy native-only subtree; it does not prove a portable graph.
Unrelated lock changes still need explicit source review in either mode.
