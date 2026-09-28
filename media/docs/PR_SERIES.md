# Review and release order

The integration entry point is [zencodec #129](https://github.com/imazen/zencodec/pull/129).
The experiment lives under `zencodec/media` and remains unpublished. Every active
codec dependency is pinned to an immutable revision in
[`repos.lock.json`](../integration/repos.lock.json). The checkout runner checks
package identities, applies registry and direct-git overrides, and refuses a
change in the locked package/version multiset.

| Layer | Changes and owner PRs |
|---|---|
| Last 0.2 pixel bridge | [zenpixels #76](https://github.com/imazen/zenpixels/pull/76): checked current sample encoding and planar deprecation |
| Breaking pixel release | [zenpixels #77](https://github.com/imazen/zenpixels/pull/77): remove planar; keep full-domain packed U16 semantics; preserve unsupported transfer codes. [#78](https://github.com/imazen/zenpixels/pull/78): convert from current ICC/CICP and perform actual output-profile conversion |
| Exact animation contract | [zencodec #127](https://github.com/imazen/zencodec/pull/127): rational duration, timed frame submission, explicit total plays |
| Native video lifecycle | [rav1d-safe #531](https://github.com/imazen/rav1d-safe/pull/531), [#532](https://github.com/imazen/rav1d-safe/pull/532), [#533](https://github.com/imazen/rav1d-safe/pull/533): metadata units, packet ownership/backpressure, presentation provenance. [zenrav1e #44](https://github.com/imazen/zenrav1e/pull/44): small-frame and lossless reconstruction fixes |
| GIF | [zengif #16–20](https://github.com/imazen/zengif/pull/20): exact delay/loops, erasure/compositing, terminal cancellation, limits and explicit color admission; [zenquant #12–13](https://github.com/imazen/zenquant/pull/13): transparent palette handling |
| APNG | [zenpng #21](https://github.com/imazen/zenpng/pull/21): composited 8/16-bit frames, rational timing and limits. [#22](https://github.com/imazen/zenpng/pull/22): native color negotiation and unknown signaling |
| WebP | [zenwebp #105](https://github.com/imazen/zenwebp/pull/105): exact timing/plays, single-frame ANIM, cancellation and context |
| JPEG XL | [zenjxl-decoder #60](https://github.com/imazen/zenjxl-decoder/pull/60), [jxl-encoder #124–127](https://github.com/imazen/jxl-encoder/pull/127), [zenjxl #20](https://github.com/imazen/zenjxl/pull/20): movable decoder, native animation/color/clock contracts and repeated 1×1 canvases |
| AVIF | [cavif-rs #7](https://github.com/imazen/cavif-rs/pull/7), [zenavif #50](https://github.com/imazen/zenavif/pull/50), [#51](https://github.com/imazen/zenavif/pull/51): native 12-bit samples, exact animation timing, borrowed cancellation, retained-frame budgets and decoded color context |
| Consumers | [zenpipe #85](https://github.com/imazen/zenpipe/pull/85): own filter channel mask. [heic #52](https://github.com/imazen/heic/pull/52): shared pixel/gain-map types. [zenjpeg #211](https://github.com/imazen/zenjpeg/pull/211), [zensim #63](https://github.com/imazen/zensim/pull/63): current PNG dependency compatibility |

Review owner fixes before the integration layer. Several PRs are stacked;
merging only the last diff of a stack is insufficient. The manifest always pins
the complete tested branch tip. Rebase/re-pin together before release and run
the integration command again. No automated merge or publication is part of
these changes. The breaking pixel branch stages **0.3.1** because 0.3.0 was
already yanked; the last 0.2 bridge is a separate branch.

10/12-bit video planes must state storage width, significant code bits, bit
placement, range, matrix, and chroma location. Original source bit depth is
provenance, not a declaration of the current buffer's numerical domain. Packed
RGB U16 still means 0–65535. A native 10/12-bit plane cannot masquerade as that
format; widening/quantization is explicit and tested.

A passing integration suite is not a substitute for each owner's release gates.
In particular, the AVIF PR records measured pre-existing RD/monotonicity gate
failures without changing expected scores or hiding them. Linux tests do not
establish Apple/Windows/Android/GPU/ARM runtime correctness. The frozen scorer
revision is deliberate; dependency work does not retrain or silently select a
new quality profile.
