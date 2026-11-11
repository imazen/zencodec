# Resident native decode trial

This is a bounded experiment with RGB8 QOI, 24-bit BMP and baseline JPEG
noise/patch witnesses, not production integrations. It compares native decoding,
an additional native header probe, the existing erased one-shot API, a borrowed
typed/dyn entry point taking `&[u8]` and `&dyn Pulse`, and the reimagined slice,
positioned and inline Tokio drivers. Construction, output ownership and drop are
included; source creation and runtime creation are outside timing.

`setup.py` exports zenbitmaps at `9d71b356` from a local checkout into ignored
vendor storage. Two experimental accessors expose its existing QOI header parser
and move out its private `Cow` of pixels. Decoding kernels are unchanged. These
accessors are required for this experiment; they are not published native APIs.
JPEG and its dependency cohort come from the retained
[PR260](https://github.com/imazen/zenjpeg/pull/260) graph, with the exact commits
in Cargo.toml/Cargo.lock. Earlier attempted graphs failed resolution or compile;
those logs are preserved in the measurement artifacts. No sibling checkout changed.

Native output allocation pointers must survive extraction and PixelBuffer
adoption. Every arm must produce identical descriptors, dimensions and samples
at 8, 63 and 256 square pixels. The borrowed Pulse control holds stack references,
refuses sharing, reaches native cancellation checks, retains native timeout
causes, and receives JPEG progress. Bitmap raw APIs currently accept Stop, so
those controls qualify cancellation, not bitmap progress reporting.

The borrowed entry point is a cost control, not a complete proposed trait. It
does not implement pre-output policy admission or the collection contract.
The WholeImage frontends are success-path measurement bindings for these
explicitly selected witnesses. Arbitrary/malformed files, metadata, color policy,
orientation, other layouts, native workspace caps, streaming uploads and full
codec conformance are not qualified here. WholeImageDecode currently has no
Pulse argument; that limitation is deliberately recorded rather than hidden by
an owned Pulse captured in a codec object.

From reimagined/, run `just native-slice-setup` once, then
`just native-slice-check` and `just native-slice-build` through run-heavy with an
isolated target directory. `native-slice-run` retains frozen zenbench evidence;
`native-slice-profile` retains all-thread heaptrack call counts at zero and
1,000 additional operations, including the same warm operation in both captures.
Borrowed cost arms use `&NoPulse`; the reporting observer is a separate behavior
control, not a measured claim about arbitrary progress observers.

`cpu-profile.py` measures a different diagnostic: inherited perf task-clock,
duration, user and system CPU counters around the retained native and synthetic
loop probes. Five shuffled blocks compare zero versus 10,000 extra in-memory
decodes, with identical fixture creation and one warm operation in each process.
The native 256-square RGB8 routes include an unchanged A/A control; synthetic
256-square RGBA8 routes include typed, boxed, inline and worker paths. CPU is
summed across threads. All samples, negative adjusted kernel-time noise and the
realized schedule are retained; profiler wall time is not zenbench confirmation.
No codec CLI is launched per decode. `profile_lease` keeps the published
zenbench exclusive lock over the complete diagnostic. The launch separately
records verified CPU placement and the resource wrapper. The existing invalid
wall-time studies remain invalid.
Use a fresh output directory. Invalid timing stays invalid; allocation calls do
not establish latency or process memory.
