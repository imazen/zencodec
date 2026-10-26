# Native AV1 policy qualification

This standalone workspace runs the shared policy resolver over real
`Av1VideoDecoder` presentations, then sends the approved native owners to
`Av1VideoEncoder`. FFmpeg/libdav1d independently reads the resulting IVF files.
Expected samples come from strided source planes; no conversion is used.

The selected-image output trial compiles the same
[`ImageOutput`](../spatial-validation/src/image_output.rs) used by the spatial
harness. `just av1-output-check <fresh-artifacts>` exercises three frames in
each of the 15 depth/layout configurations with native encode/decode and
required FFmpeg readback. It preserves native plane addresses, code depth,
roles, color/siting/crop/epoch interpretation and presentation timing, survives
decoder reset/drop, and releases the picture with the final output owner.
Its packed-color convenience refuses planar code samples. The trial is private;
it does not establish a display conversion or a complete memory bound.

The output gate passed on 2026-10-26: run-heavy rc=0, 9 s, peak RSS 0.54 GiB,
minimum available 13206 MiB, peak load 0.36. Log:
`~/tmp/zencodec-common-output-video1-2026-10-26.log`. This measures the build/test
process. Native picture-address and ownership assertions establish the storage
handoff, rather than a process-memory estimate.

From an export outside the zen workspace's path overrides:

```sh
just av1-policy-setup
just av1-policy-check /path/to/fresh/persistent/artifacts
```

FFmpeg with libdav1d and the `timeout` command are required. Each oracle process
has a 30-second deadline. Existing artifacts are never overwritten: choose a
fresh directory for each run. The Cargo manifest pins the native PRs and patches
all shared zencodec types to this tree. `dependencies.lock` records resolution.

CI selects Ubuntu's archive mirror in both direct entries and the runner's
`apt-mirrors.txt`, keeping the signed FFmpeg package, with separate two-minute
update and three-minute installation deadlines. The initial
Azure mirror attempt exhausted the job's 15-minute deadline before tests began
(run 40237893129). Installation failure fails qualification; it never skips the
independent decoder checks. Run 40241228768 exposed the mirror-list indirection:
the earlier direct-entry edit left downloads on Azure and the installation
deadline correctly failed with status 124. Both configuration paths are now
rewritten; HTTP and HTTPS transfers also have per-transfer deadlines.

The matrix covers 8/10/12-bit mono, YUV 4:2:0, 4:2:2, 4:4:4 and identity-matrix
GBR, each at 17x13 with padded input strides, in both ordinary pooled and
bounded native-plane modes. Both modes independently re-encode and pass exact
FFmpeg/libdav1d readback. Other tests check queued epoch
changes at one/four native threads, multiple presentations per packet, repeated
packet ordinals, exact picture ownership, native and callback error traces,
refusal, stale choices, cancellation and caught callback panics.

The executable guide now configures `max_picture_bytes` before opening the
native session. Refusal precedes the publication callback, drops the native
decoder and retains the original `At<rav1d_safe::Error>` through the policy
adapter. Accepted native frames keep their storage allowance after decoder
drop. The native setting bounds requested plane bytes, including padding and
alignment; it excludes input, metadata, codec workspace and allocator overhead.
The native error is still `Error::Other` with a trace; structured refusal facts
and a shared resource category remain open.

Publication policy still runs after native reconstruction. This does not
establish a complete decoder RAM bound, pre-reconstruction metadata suspension,
container privacy, planar conversion, all AV1 profiles, encoder output scheduling
or performance.
The native decoder's broader conformance and film-grain tests live in
[rav1d-safe PR 537](https://github.com/imazen/rav1d-safe/pull/537); the session
adapter is [zenavif PR 74](https://github.com/imazen/zenavif/pull/74).

Frame/IVF fixture helpers are adapted from zenavif `ea055f7d`'s
`tests/media_session.rs` and `examples/media_ivf.rs`. They remain test code.

The native budget integration passed on 2026-10-25 with zenavif `feadc4c9`
and rav1d-safe `b19aec65`. Its 15 depth/layout configurations pass in both
allocation modes, alongside the refusal/trace/lifetime tests and strict
Clippy. [Qualification receipt](../../benchmarks/native_picture_policy_2026-10-25.txt):
run-heavy rc=0, 29 s, peak RSS 1.13 GiB, minimum available 15274 MiB,
peak load 1.68. This is build/test-process usage, not a decoder memory or speed
measurement.
