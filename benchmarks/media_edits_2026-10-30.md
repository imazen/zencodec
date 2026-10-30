# Native MP4 edit-list information loss

Source: `07573a469abc5989f333ac1723b3c9fe2485006d`.
Archive SHA-256: `d96d226a53aa20c876e18e470e050ec4c23ec5d40b4869b1d2494e4757031acd`.
Unchanged AV1 harness lock SHA-256:
`fa963796ba906c475546f591cae3fae3483fb84f3ad41d39588de4e11176292c`.
Native zenmp4: zenavif revision `94116d266299f347cfd10a75d440a5bc3b598d2c`.

Command: `just av1-policy-setup && just media-edit-list-probe`, with the Cargo
command run under a 3 GiB, two-job cap and a 900-second timeout in an isolated
source export. Debug information and incremental compilation were disabled.
The archive and lock hashes were reverified after execution. Source compilation
was observed; the supervisor and Cargo process exited and no Rust workers
remained. Complete local log: `~/tmp/mp4-edit-probe-20261030/full.log`.

The native muxer authored a fragmented initialization segment with an explicit
AV1 configuration record. The probe added version-0 edit lists to the track.
It did not encode/decode samples. The native demuxer accepted both controls:
no edits produced `None`, and a single edit starting at media time 17 produced
`Some(17)`. It then accepted all three distinct programs with the same result:

| Change from the single-edit control | Demuxer edit offset |
| --- | --- |
| Duration 1000 → 2000 movie ticks | `Some(17)` |
| Append a second media segment starting at 51 | `Some(17)` |
| Playback rate 1 → 2 | `Some(17)` |

The diagnostic exited 101 at its final assertion, listing all three information
losses. This is an intentionally failing witness, not a passing regression test.
Resource receipt: `rc=101 61s | peak-RSS 0.78GiB | min-avail 13914MiB | peak-load 1.31`.
These are build/run resources, not codec performance measurements.

Strict Clippy for this example passed on the same frozen source and lock:
`cargo clippy --manifest-path perf/av1-policy-validation/Cargo.toml --locked
--example edit_list_probe -- -D warnings`.
Receipt: `rc=0 25s | peak-RSS 0.65GiB | min-avail 14117MiB | peak-load 1.42`.
This is scoped example qualification, not the full native harness suite.

## Contract consequence

One scalar cannot distinguish these programs. Merely copying that scalar into
`CodecConfig` cannot fix edited presentation or faithful remuxing. Retain the
ordered presentation map, including empty intervals, segment durations and
source starts; represent supported playback rates or refuse unsupported ones.
Keep codec priming and seek pre-roll distinct from timeline mapping. Apply a
map once in the presentation adapter; raw packet decoding must not silently
claim to have applied it.

Still unqualified: independent-player playback, clipping across decoded-frame
boundaries, leading empty edits, seek/reset/configuration epochs, negative
origins, and any replacement API. This receipt establishes native information
loss only. Follow-up: [issue 187](https://github.com/imazen/zencodec/issues/187).
