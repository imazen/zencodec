# Multi-pass input contract experiment

This excluded harness tests a candidate driver and encoder interface. It is not
an implementation of the production zencodec traits. The starting driver and
native adapter came from Opus review revision `0ec4726`; independent review
replaced its detached replay token with a driver-retained raster input.

```rust,ignore
// Freeze and retain actual pixels, including stride and color context.
let input = multipass_validation::Input::raster(pixels);
// Or use a forward/positional source; its callback cannot claim borrowed proof.
let input = multipass_validation::Input::source(source);
```

The driver reads the retained raster itself on every pass. External source
replay uses explicitly disclosed digest/version evidence, or an admitted owned
snapshot. Digests detect accidental changes; they do not prove exact equality.
Exact replay permits only no replay, the retained immutable raster, or driver-owned
memory staging. Source pass budgets apply to every route, including staging.
When a budget requires staging, the replay callback can refuse its cost before
any pixel read. A direct raster that needs no staging makes no digest or raw
staging reservation.

Run public controls with `just multipass-check`. The isolated native crate
requires access to the private IMI repository and is explicitly opt-in:
`just multipass-native-check <artifact-directory>`. It calls the native codec's
public session API; no private codec implementation is copied here. Its pinned
graph contains a separate zencodec copy used inside IMI; no zencodec types cross
that boundary. The prototype and adapter share the current local interface.

The public controls exercise protocol misuse, swallowed interruptions, replay
changes, staging choices, pass limits, detached-proof substitution and direct
raster retention. Native IMI gates exercise byte/sample parity, padded rows,
forward and seek-back output, positional/disk/memory replay, source mutation,
cancellation, partial I/O and unsupported interpretation. Native full-suite and
strict-lint receipt (2026-10-27):
`rc=0 13s | peak-RSS 0.67GiB | min-avail 8289MiB | peak-load 0.95`.
Log: `~/tmp/zencodec-retained-raster-pass-budget-20261027.log`.
These are compilation/test resources, not codec performance or memory bounds.

Remaining contracts: merge replay decisions into the single PolicyResolver;
compose mandatory requirement scope across replay and output scheduling; unify
single-pass and multi-pass source/destination drivers; provide a bounded
production diagnostic trace; qualify total retention and asynchronous source
lifetimes. Zero zenpixels files changed for the exercised paths. Pixel-aligned
strides remain required for PixelSlice; the file source handles unaligned file
row strides internally.
