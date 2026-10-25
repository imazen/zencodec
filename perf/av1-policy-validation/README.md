# Native AV1 policy qualification

This standalone workspace runs the shared policy resolver over real
`Av1VideoDecoder` presentations, then sends the approved native owners to
`Av1VideoEncoder`. FFmpeg/libdav1d independently reads the resulting IVF files.
Expected samples come from strided source planes; no conversion is used.

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
GBR, each at 17x13 with padded input strides. Other tests check queued epoch
changes at one/four native threads, multiple presentations per packet, repeated
packet ordinals, exact picture ownership, native and callback error traces,
refusal, stale choices, cancellation and caught callback panics.

This qualifies publication after native reconstruction. It does not establish
native memory bounds, pre-reconstruction metadata suspension, container privacy,
planar conversion, all AV1 profiles, encoder output scheduling or performance.
The native decoder's broader conformance and film-grain tests live in
[rav1d-safe PR 537](https://github.com/imazen/rav1d-safe/pull/537); the session
adapter is [zenavif PR 74](https://github.com/imazen/zenavif/pull/74).

Frame/IVF fixture helpers are adapted from zenavif `ea055f7d`'s
`tests/media_session.rs` and `examples/media_ivf.rs`. They remain test code.
