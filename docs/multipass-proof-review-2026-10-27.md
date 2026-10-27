# Replay proof must identify the actual retained source

Independent review of the multipass prototype at `0ec4726` found that
`BorrowProof` can be detached from the source it is supposed to establish.
Its private constructor prevents direct construction, but `PassSource::replay()`
returns a copyable `Replay::Borrowed(proof)` without tying that token to source
identity or lifetime. A different callback source can return the same token.

The reproduction in [multipass-proof-substitution.rs](../perf/multipass-proof-substitution.rs)
obtains a token from a real borrowed raster, destroys that raster, and gives the
token to a source whose samples change between passes. With exact replay required
and memory staging forbidden, the prototype accepts both passes and publishes.
No unsafe code or native codec is involved; the real prototype driver executes
against a minimal two-pass encoder that records the supplied samples.

Observed failure:

```text
detached proof accepted different pass samples [0, 1] and published [1]
```

Reproduction: export `perf/multipass-validation` from review revision `0ec4726`,
copy the linked file to its `tests/proof_substitution.rs`, and run
`cargo test --manifest-path perf/multipass-validation/Cargo.toml --test proof_substitution`.
The desired-contract assertion fails on that revision. Log:
`~/tmp/zencodec-replay-proof-substitution-20261027.log`.
Wrapper receipt: `rc=101 6s | peak-RSS 0.34GiB | min-avail 8577MiB | peak-load 0.34`.
This is a correctness reproduction and build/test resource receipt, not a speed
or codec-memory measurement.

## Contract consequence

Exact borrowed replay must be a driver-held immutable pixel view that the driver
itself reads. A reusable marker, lifetime-free identity, private constructor, or
non-cryptographic digest cannot establish that an arbitrary callback supplies
those pixels. Merely removing `Copy` is insufficient: forwarding a legitimate
token still does not bind the callback's reads to its original source.

A candidate design is a concrete raster source path holding `PixelSlice`, with
callbacks as separate forward/positional routes. The raster path must freeze the
actual view once and use it throughout every pass. Repeatedly asking a callback
for a fresh immutable view is insufficient because it can select different views.
This also allows the driver to derive borrowed-raster ownership for output
scheduling from the retained view rather than a source's bare claim.

The implementation still needs testing for returned-view substitution, retained
source lifetime through output waits, strided rows, color context, cancellation,
source/destination failures, and native IMI parity. The prototype's other protocol
and native tests do not close this finding. Its public integration remains open.
