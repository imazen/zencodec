# Native RGB8 mechanism trial

QOI, BMP and baseline 4:2:0 JPEG use the same native decoding kernels through
resident, positioned, streamed and Tokio routes. `None` disables Pulse. This is
an experiment binding, not a complete production codec migration: metadata,
non-RGB8 output and general native error classification are not qualified.
Native stop reasons and concrete error causes are preserved. The whole-image
header is `()` here; JPEG therefore still parses its header twice. The contract
allows retained preparation but this binding does not demonstrate it.

Source adapted from zencodec `05b73f5f4e26a45c4bb4f584b590831117bb36a2`.
`setup.py` exports zenbitmaps `9d71b356f290f6438b810cc53d99f29301f6d247`
and adds only two accessors: a QOI probe and native output ownership extraction.
`verify-source.py` checks every original tracked source blob against that commit.
No native decoder kernel changes. The BMP probe treats native EOF/amplification
refusals on a prefix as a request for more bytes. It still does not qualify
general malformed-file classification, because the hook has no EOF argument. JPEG is pinned in Cargo.toml and the tracked
`dependencies.lock` preserves the complete dependency graph.

The older erased bitmap adapter copies the complete decoded output in
`layout_to_pixel_buffer`. Its extra time is not solely a trait comparison.
`qualify` checks exact native samples, dimensions and descriptors for every arm,
and pointer identity across native output extraction and PixelBuffer adoption.
The borrowed observer control requires an unshareable observer to reach native
checks and JPEG reporting, retaining the original native timeout cause.

From `reimagined/`, run `python3 tools/native-slice/setup.py`, then the
`probe-check` and `probe-build` recipes with `PROBE_TARGET` pointing to an
isolated output directory. `allocations fixture CODEC SIZE FILE` saves a fixture;
`allocations CODEC qualify @FILE 0` checks every route;
`allocations CODEC ARM @FILE COUNT` warms once and performs COUNT extra decodes.
No timer is inserted into codec code. The large-study controller lives in
`../large-decode/`.
