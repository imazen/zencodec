# Reimagined document experiment

Unpublished standalone workspace; this is not the released zencodec API.
[README](README.md), [design](docs/document-design.md), and
[validation](docs/validation-2026-11-11.md) own its scope and evidence.

## Known Bugs found and fixed in the candidate

- Opaque IDs need pointer identity for their namespace; comparing `Arc<()>`
  values aliases distinct origins. The origin control prevents that.
- A native Done could bypass final rows/finish, or swallow a sink error. The
  selected-publication guard retains coverage, terminal state and typed errors.
- Pre-cancelled queries/selections left the native owner retained. Both now
  close before further source work; native in-flight latency remains unqualified.
- The validation worker returned Interrupted when its transport closed.
  `read_exact` retried indefinitely and Drop joined the spinning worker.
  Closure now returns BrokenPipe; the native source-failure control checks cause
  preservation and completed teardown.

## Open qualification gaps

Only the bounded synthetic and tiled Gray8 TIFF witnesses execute. Native EXR,
JXL and IMIJ, arbitrary malformed directory graphs, async document driving,
complete metadata, output-storage budgets, native memory bounds, rendering and
performance are not established. Keep these explicit when extending the fork.
