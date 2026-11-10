# Native frame contract witnesses

`just frame-interchange-native-check INPUTS` runs from an isolated repository
export under the resource wrapper. It fetches two SHA-verified AVIF fixtures from
an immutable zenavif revision and records the installed FFmpeg version, exact
commands and output hashes. Missing inputs/oracles are failures, not skips.
The tracked dependency seed is `dependencies.lock`.

The AV1 witness keeps native rav1d plane guards alive, reports their actual
per-plane encoding and lends them through the checked frame/image/planar paths.
Pictures outlive the decoder and match the independent reader exactly for the
selected odd-size 8-bit and 10-bit monochrome cases. The mapping adapter is
adapted from zenavif media mapping; no decoder kernel is copied or changed.
The TIFF witness decodes two pages with different odd widths/depths and runs the
same reusable checker. These are interchange witnesses, not a complete video
session, TIFF collection/SubIFD or independently verified edited-playback suite.

The shared testkit separately checks padded/minimal-final-row, mixed-depth
semiplanar alpha, MSB 10-bit, unshifted 12-bit and F32 bit patterns including
NaN payloads and signed zero. No source-depth hint certifies lower-depth encoding.

Frozen spatial Mox/LCMS numerical comparison cohorts remain separate. This new
minimal graph uses the selected core-stack pixel revision and does not rewrite
those earlier reference failures or thresholds.
