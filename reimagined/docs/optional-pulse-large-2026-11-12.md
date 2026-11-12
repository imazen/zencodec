# Optional Pulse and 4096² / 8192² decode drivers

Missing: current-head allocation counts, native metadata/general-error conformance, tile or ROI browsing, actual disk/network throughput and concurrent-runtime fairness. The native bindings remain RGB8 experiments. No production codec is migrated or released.

The borrowed slice path and optional borrowed observer are implemented. Decoders still require `Send + 'static`; a decoder that retains a source borrow between steps, or is thread-bound, remains outside this contract. `None` disables observation; `Some(&pulse)` retains reporting and cancellation. Worker paths accept `Option<SharedPulse>`. The borrowed, unshareable observer control reaches native checks and JPEG reporting. Default fixture paths exercise `None` without constructing a shared observer.

Exact pixels, dimensions and descriptors pass through all 15 native paths at both sizes for QOI, BMP and baseline JPEG. Synthetic RGBA8 passes all 19 paths at both sizes with tightly packed and 16-byte-padded rows, exact row coverage, completion and unchanged padding. Native output extraction/adoption retains pointer identity.

## What the measurements say

At 8192² the boxed resident path’s difference from native-plus-probe is unresolved against A/A variation for all three codecs. This is not a claim of zero overhead. At 4096² BMP is an exception: the boxed path has a positive diagnostic contrast just beyond the observed spread.

For an existing QOI input owner at 8192², one-hop wall / CPU is 183.846 / 183.818 ms; generic pool is 302.343 / 302.180 ms. The generic pool source here is a borrowed InMemory slice and uses its owned input window. The one-hop source is an existing Arc owner, handed to the worker without copying its bytes.

For an existing BMP input owner at 8192², one-hop wall / CPU is 110.853 / 110.803 ms; generic pool is 197.984 / 197.875 ms. The generic pool source here is a borrowed InMemory slice and uses its owned input window. The one-hop source is an existing Arc owner, handed to the worker without copying its bytes.

For an existing JPEG input owner at 8192², one-hop wall / CPU is 308.568 / 308.474 ms; generic pool is 329.106 / 329.022 ms. The generic pool source here is a borrowed InMemory slice and uses its owned input window. The one-hop source is an existing Arc owner, handed to the worker without copying its bytes.

Synthetic 4096² pool wall / CPU: older 30.394 / 30.397 ms; newer 31.278 / 31.268 ms. A/A spreads are 0.683 / 0.674 ms: wall beyond A/A spread, CPU beyond A/A spread. The earlier pool regression cannot be declared removed by the window repair.

Synthetic 8192² pool wall / CPU: older 119.049 / 118.964 ms; newer 121.803 / 121.683 ms. A/A spreads are 4.012 / 3.988 ms: wall unresolved, CPU unresolved. The earlier pool regression cannot be declared removed by the window repair.

Keep the resident slice and input-owner routes as direct conveniences. Use the input-owner route for resident offload; this evidence does not justify routing it through the generic source window. InPlace fairness and actual tile browsing still need their own evidence; this bounded phase does not implement or qualify them.

## Method and limits

Five shuffled blocks, paired zero-extra process with identical file load/runtime/one warm decode. Fixed doubling calibration from one decode until adjusted loop >=500ms. Native AA and synthetic direct_AA unchanged. None Pulse on every path.

External perf diagnostic; no zenbench Confirm verdict. Every pair and negative counter retained. Memory-backed Streamed and Cursor sources, no disk or network throughput claim. InPlace called from Runtime::block_on outside a worker task; no fairness/worker-replacement claim.

Each value is the arithmetic mean of five baseline-subtracted blocks. A/A spread is the maximum absolute same-block reference-versus-identical-control difference; it is an observed variation bound, not a confidence interval. Differences within that spread are unresolved. Exceeding it does not establish a clean latency acceptance result. Negative kernel counters remain in the evidence.

Both native and synthetic probes use Tokio multi-thread runtimes with two workers. The older and newer arms within each binary share one dependency graph. These are different probe binaries; compare paths within a binary rather than attributing cross-binary differences to the contract. The synthetic shape uses 64-row strips and a 64-step quota at both sizes. This differs from the earlier 1024² study’s 16-row strips.

Native `next_slice` drives the boxed protocol into a minimal owned-frame sink. The user-facing `memory::decode` wrapper additionally collects frames and clones info; its complete end-to-end overhead is not timed here. `borrowed_dyn` is the box-free measurement entry. JPEG still parses its header twice on the whole-image binding (`Header=()`); the hook permits reuse but this binding does not demonstrate it. The older erased BMP/QOI adapters copy the decoded output; their complete delta is not trait overhead.

## Native results

Times in milliseconds/decode. N is extra decodes/process. Spread is wall / CPU milliseconds. Streamed and Cursor sources are memory-backed. `native_AA` executes native unchanged.

| Codec / size | Route | N | Wall | CPU | Kernel | Switches | A/A wall / CPU |
|---|---|---:|---:|---:|---:|---:|---:|
| qoi 4096² | native | 16 | 45.123 | 45.111 | 13.835 | 1.025 | 1.892 / 1.886 |
| qoi 4096² | native_probed | 16 | 44.815 | 44.806 | 13.840 | 0.900 | 1.892 / 1.886 |
| qoi 4096² | borrowed_typed | 16 | 44.465 | 44.471 | 13.127 | 0.450 | 1.892 / 1.886 |
| qoi 4096² | borrowed_dyn | 16 | 44.971 | 44.962 | 13.852 | 0.738 | 1.892 / 1.886 |
| qoi 4096² | older_erased | 8 | 69.242 | 69.242 | 28.482 | 1.175 | 1.892 / 1.886 |
| qoi 4096² | next_slice | 16 | 44.800 | 44.788 | 13.798 | 1.075 | 1.892 / 1.886 |
| qoi 4096² | next_read_at | 16 | 44.840 | 44.824 | 13.847 | 0.512 | 1.892 / 1.886 |
| qoi 4096² | next_inline | 16 | 44.862 | 44.855 | 13.410 | 0.887 | 1.892 / 1.886 |
| qoi 4096² | next_in_place | 16 | 44.633 | 44.622 | 13.472 | 0.775 | 1.892 / 1.886 |
| qoi 4096² | next_pool | 8 | 75.157 | 75.090 | 36.736 | 6.050 | 1.892 / 1.886 |
| qoi 4096² | next_buffered_one_hop | 16 | 46.661 | 46.624 | 16.243 | 3.050 | 1.892 / 1.886 |
| qoi 4096² | next_streamed_inline | 8 | 72.052 | 72.034 | 34.401 | 1.500 | 1.892 / 1.886 |
| qoi 4096² | next_streamed_pool | 8 | 75.313 | 75.269 | 36.404 | 6.350 | 1.892 / 1.886 |
| qoi 4096² | next_streamed_in_place | 8 | 71.962 | 71.955 | 33.648 | 1.050 | 1.892 / 1.886 |
| qoi 4096² | next_seek_read | 8 | 72.005 | 71.982 | 33.847 | 1.275 | 1.892 / 1.886 |
| qoi 4096² | native_AA | 16 | 44.842 | 44.828 | 13.346 | 1.113 | 1.892 / 1.886 |
| qoi 8192² | native | 4 | 182.427 | 182.424 | 58.286 | 2.450 | 2.969 / 2.943 |
| qoi 8192² | native_probed | 4 | 182.336 | 182.311 | 58.797 | 2.400 | 2.969 / 2.943 |
| qoi 8192² | borrowed_typed | 4 | 182.722 | 182.697 | 60.945 | 3.100 | 2.969 / 2.943 |
| qoi 8192² | borrowed_dyn | 4 | 184.269 | 184.235 | 61.033 | 3.250 | 2.969 / 2.943 |
| qoi 8192² | older_erased | 2 | 284.940 | 284.967 | 117.510 | 3.200 | 2.969 / 2.943 |
| qoi 8192² | next_slice | 4 | 183.115 | 183.040 | 59.126 | 3.700 | 2.969 / 2.943 |
| qoi 8192² | next_read_at | 4 | 185.008 | 184.934 | 61.236 | 4.450 | 2.969 / 2.943 |
| qoi 8192² | next_inline | 4 | 181.966 | 181.925 | 59.098 | 2.350 | 2.969 / 2.943 |
| qoi 8192² | next_in_place | 4 | 183.366 | 183.395 | 59.111 | 2.400 | 2.969 / 2.943 |
| qoi 8192² | next_pool | 2 | 302.343 | 302.180 | 149.816 | 13.700 | 2.969 / 2.943 |
| qoi 8192² | next_buffered_one_hop | 4 | 183.846 | 183.818 | 62.783 | 4.250 | 2.969 / 2.943 |
| qoi 8192² | next_streamed_inline | 2 | 297.537 | 297.440 | 143.480 | 6.100 | 2.969 / 2.943 |
| qoi 8192² | next_streamed_pool | 2 | 293.963 | 293.873 | 137.714 | 9.800 | 2.969 / 2.943 |
| qoi 8192² | next_streamed_in_place | 2 | 293.666 | 293.584 | 142.588 | 3.100 | 2.969 / 2.943 |
| qoi 8192² | next_seek_read | 2 | 293.644 | 293.520 | 141.061 | 3.500 | 2.969 / 2.943 |
| qoi 8192² | native_AA | 4 | 182.733 | 182.727 | 59.851 | 1.600 | 2.969 / 2.943 |
| bmp 4096² | native | 32 | 26.347 | 26.341 | 13.051 | 0.550 | 0.430 / 0.422 |
| bmp 4096² | native_probed | 32 | 26.112 | 26.107 | 13.091 | 0.394 | 0.430 / 0.422 |
| bmp 4096² | borrowed_typed | 32 | 26.343 | 26.339 | 13.172 | 0.456 | 0.430 / 0.422 |
| bmp 4096² | borrowed_dyn | 32 | 26.641 | 26.631 | 13.698 | 0.500 | 0.430 / 0.422 |
| bmp 4096² | older_erased | 16 | 51.709 | 51.702 | 28.553 | 0.900 | 0.430 / 0.422 |
| bmp 4096² | next_slice | 32 | 26.606 | 26.599 | 13.733 | 0.556 | 0.430 / 0.422 |
| bmp 4096² | next_read_at | 32 | 26.615 | 26.611 | 13.967 | 0.456 | 0.430 / 0.422 |
| bmp 4096² | next_inline | 32 | 26.481 | 26.474 | 13.601 | 0.556 | 0.430 / 0.422 |
| bmp 4096² | next_in_place | 32 | 26.411 | 26.406 | 13.267 | 0.556 | 0.430 / 0.422 |
| bmp 4096² | next_pool | 16 | 50.289 | 50.249 | 32.074 | 5.625 | 0.430 / 0.422 |
| bmp 4096² | next_buffered_one_hop | 32 | 29.200 | 29.186 | 16.130 | 2.394 | 0.430 / 0.422 |
| bmp 4096² | next_streamed_inline | 16 | 47.231 | 47.224 | 28.486 | 0.475 | 0.430 / 0.422 |
| bmp 4096² | next_streamed_pool | 16 | 49.811 | 49.773 | 30.670 | 5.037 | 0.430 / 0.422 |
| bmp 4096² | next_streamed_in_place | 16 | 47.379 | 47.372 | 28.519 | 0.438 | 0.430 / 0.422 |
| bmp 4096² | next_seek_read | 16 | 48.744 | 48.735 | 30.343 | 0.713 | 0.430 / 0.422 |
| bmp 4096² | native_AA | 32 | 26.382 | 26.376 | 13.648 | 0.500 | 0.430 / 0.422 |
| bmp 8192² | native | 8 | 109.433 | 109.418 | 59.738 | 2.225 | 3.274 / 3.298 |
| bmp 8192² | native_probed | 8 | 109.255 | 109.235 | 60.043 | 1.300 | 3.274 / 3.298 |
| bmp 8192² | borrowed_typed | 8 | 107.835 | 107.809 | 58.414 | 1.475 | 3.274 / 3.298 |
| bmp 8192² | borrowed_dyn | 8 | 110.122 | 110.086 | 59.397 | 1.925 | 3.274 / 3.298 |
| bmp 8192² | older_erased | 4 | 209.607 | 209.554 | 119.188 | 2.850 | 3.274 / 3.298 |
| bmp 8192² | next_slice | 8 | 109.449 | 109.421 | 58.986 | 1.825 | 3.274 / 3.298 |
| bmp 8192² | next_read_at | 8 | 109.019 | 108.985 | 59.710 | 2.425 | 3.274 / 3.298 |
| bmp 8192² | next_inline | 8 | 109.706 | 109.668 | 59.990 | 2.000 | 3.274 / 3.298 |
| bmp 8192² | next_in_place | 8 | 109.826 | 109.820 | 58.652 | 1.350 | 3.274 / 3.298 |
| bmp 8192² | next_pool | 4 | 197.984 | 197.875 | 127.912 | 12.500 | 3.274 / 3.298 |
| bmp 8192² | next_buffered_one_hop | 8 | 110.853 | 110.803 | 61.176 | 4.400 | 3.274 / 3.298 |
| bmp 8192² | next_streamed_inline | 4 | 193.714 | 193.703 | 121.079 | 2.950 | 3.274 / 3.298 |
| bmp 8192² | next_streamed_pool | 4 | 197.299 | 197.183 | 121.025 | 12.300 | 3.274 / 3.298 |
| bmp 8192² | next_streamed_in_place | 4 | 193.782 | 193.742 | 121.480 | 3.600 | 3.274 / 3.298 |
| bmp 8192² | next_seek_read | 4 | 195.577 | 195.544 | 121.263 | 2.700 | 3.274 / 3.298 |
| bmp 8192² | native_AA | 8 | 108.959 | 108.920 | 58.341 | 1.950 | 3.274 / 3.298 |
| jpeg 4096² | native | 8 | 75.630 | 75.604 | 13.706 | 1.225 | 2.722 / 2.749 |
| jpeg 4096² | native_probed | 8 | 76.009 | 75.994 | 13.993 | 1.075 | 2.722 / 2.749 |
| jpeg 4096² | borrowed_typed | 8 | 76.135 | 76.121 | 13.759 | 1.250 | 2.722 / 2.749 |
| jpeg 4096² | borrowed_dyn | 8 | 76.253 | 76.241 | 13.626 | 1.525 | 2.722 / 2.749 |
| jpeg 4096² | older_erased | 8 | 75.540 | 75.522 | 13.826 | 1.325 | 2.722 / 2.749 |
| jpeg 4096² | next_slice | 8 | 76.116 | 76.082 | 14.363 | 1.475 | 2.722 / 2.749 |
| jpeg 4096² | next_read_at | 8 | 76.278 | 76.267 | 13.861 | 1.450 | 2.722 / 2.749 |
| jpeg 4096² | next_inline | 8 | 76.364 | 76.340 | 14.347 | 1.275 | 2.722 / 2.749 |
| jpeg 4096² | next_in_place | 8 | 76.426 | 76.397 | 13.707 | 1.400 | 2.722 / 2.749 |
| jpeg 4096² | next_pool | 8 | 78.923 | 78.880 | 15.438 | 6.950 | 2.722 / 2.749 |
| jpeg 4096² | next_buffered_one_hop | 8 | 78.535 | 78.460 | 16.152 | 4.475 | 2.722 / 2.749 |
| jpeg 4096² | next_streamed_inline | 8 | 77.707 | 77.701 | 13.894 | 1.400 | 2.722 / 2.749 |
| jpeg 4096² | next_streamed_pool | 8 | 78.915 | 78.852 | 14.836 | 6.700 | 2.722 / 2.749 |
| jpeg 4096² | next_streamed_in_place | 8 | 77.050 | 77.042 | 13.376 | 1.325 | 2.722 / 2.749 |
| jpeg 4096² | next_seek_read | 8 | 76.551 | 76.530 | 13.747 | 1.550 | 2.722 / 2.749 |
| jpeg 4096² | native_AA | 8 | 76.244 | 76.233 | 14.090 | 1.550 | 2.722 / 2.749 |
| jpeg 8192² | native | 2 | 305.837 | 305.800 | 58.079 | 4.000 | 4.502 / 4.512 |
| jpeg 8192² | native_probed | 2 | 308.994 | 308.935 | 62.367 | 7.100 | 4.502 / 4.512 |
| jpeg 8192² | borrowed_typed | 2 | 307.252 | 307.184 | 58.831 | 3.500 | 4.502 / 4.512 |
| jpeg 8192² | borrowed_dyn | 2 | 306.731 | 306.645 | 58.971 | 3.700 | 4.502 / 4.512 |
| jpeg 8192² | older_erased | 2 | 306.421 | 306.356 | 66.366 | 4.700 | 4.502 / 4.512 |
| jpeg 8192² | next_slice | 2 | 307.540 | 307.380 | 62.039 | 3.200 | 4.502 / 4.512 |
| jpeg 8192² | next_read_at | 2 | 309.708 | 309.638 | 61.620 | 6.000 | 4.502 / 4.512 |
| jpeg 8192² | next_inline | 2 | 306.820 | 306.782 | 60.686 | 5.100 | 4.502 / 4.512 |
| jpeg 8192² | next_in_place | 2 | 306.393 | 306.315 | 62.670 | 4.800 | 4.502 / 4.512 |
| jpeg 8192² | next_pool | 2 | 329.106 | 329.022 | 76.516 | 8.200 | 4.502 / 4.512 |
| jpeg 8192² | next_buffered_one_hop | 2 | 308.568 | 308.474 | 62.150 | 5.100 | 4.502 / 4.512 |
| jpeg 8192² | next_streamed_inline | 2 | 326.837 | 326.784 | 75.283 | 5.400 | 4.502 / 4.512 |
| jpeg 8192² | next_streamed_pool | 2 | 327.528 | 327.499 | 76.094 | 14.200 | 4.502 / 4.512 |
| jpeg 8192² | next_streamed_in_place | 2 | 326.186 | 326.141 | 69.126 | 4.200 | 4.502 / 4.512 |
| jpeg 8192² | next_seek_read | 2 | 325.386 | 325.301 | 77.320 | 5.400 | 4.502 / 4.512 |
| jpeg 8192² | native_AA | 2 | 308.559 | 308.430 | 60.947 | 3.200 | 4.502 / 4.512 |

## Synthetic results

Times in milliseconds/decode; source is RGBA8 with 64-row strips and quota 64. `direct_AA` executes direct unchanged.

| Size | Route | N | Wall | CPU | Kernel | Switches | A/A wall / CPU |
|---|---|---:|---:|---:|---:|---:|---:|
| 4096² | direct | 32 | 26.953 | 26.946 | 18.730 | 0.487 | 0.683 / 0.674 |
| 4096² | direct_AA | 32 | 26.893 | 26.883 | 18.627 | 0.412 | 0.683 / 0.674 |
| 4096² | older_slice_dyn_stack | 32 | 26.609 | 26.605 | 18.048 | 0.625 | 0.683 / 0.674 |
| 4096² | older_slice_typed | 32 | 26.495 | 26.489 | 18.204 | 0.475 | 0.683 / 0.674 |
| 4096² | older_slice_dyn | 32 | 26.784 | 26.780 | 18.572 | 0.394 | 0.683 / 0.674 |
| 4096² | next_slice_boxed | 32 | 26.751 | 26.744 | 18.330 | 0.669 | 0.683 / 0.674 |
| 4096² | older_read_at | 32 | 27.010 | 27.005 | 18.415 | 0.431 | 0.683 / 0.674 |
| 4096² | next_read_at | 32 | 26.986 | 26.983 | 18.703 | 0.269 | 0.683 / 0.674 |
| 4096² | older_coroutine | 32 | 30.341 | 30.334 | 18.714 | 0.487 | 0.683 / 0.674 |
| 4096² | older_tokio_inline | 32 | 27.252 | 27.247 | 18.105 | 0.531 | 0.683 / 0.674 |
| 4096² | next_tokio_inline | 32 | 26.656 | 26.649 | 18.278 | 0.519 | 0.683 / 0.674 |
| 4096² | older_tokio_pool | 32 | 30.394 | 30.397 | 21.027 | 132.838 | 0.683 / 0.674 |
| 4096² | next_tokio_pool | 32 | 31.278 | 31.268 | 21.527 | 130.744 | 0.683 / 0.674 |
| 4096² | next_buffered_one_hop | 32 | 30.545 | 30.533 | 21.474 | 2.612 | 0.683 / 0.674 |
| 4096² | next_tokio_in_place | 32 | 26.868 | 26.866 | 18.615 | 0.331 | 0.683 / 0.674 |
| 4096² | next_streamed_inline | 32 | 27.220 | 27.212 | 18.072 | 0.512 | 0.683 / 0.674 |
| 4096² | next_streamed_pool | 16 | 31.155 | 31.149 | 21.889 | 131.050 | 0.683 / 0.674 |
| 4096² | next_streamed_in_place | 32 | 26.975 | 26.965 | 18.096 | 0.469 | 0.683 / 0.674 |
| 4096² | next_seek_read | 32 | 27.218 | 27.213 | 18.586 | 0.350 | 0.683 / 0.674 |
| 8192² | direct | 8 | 117.316 | 117.289 | 79.647 | 2.425 | 4.012 / 3.988 |
| 8192² | direct_AA | 8 | 117.068 | 117.022 | 80.780 | 1.575 | 4.012 / 3.988 |
| 8192² | older_slice_dyn_stack | 8 | 116.641 | 116.613 | 80.782 | 2.000 | 4.012 / 3.988 |
| 8192² | older_slice_typed | 8 | 116.262 | 116.251 | 79.989 | 1.950 | 4.012 / 3.988 |
| 8192² | older_slice_dyn | 8 | 118.279 | 118.245 | 81.913 | 2.300 | 4.012 / 3.988 |
| 8192² | next_slice_boxed | 8 | 116.224 | 116.225 | 80.579 | 1.750 | 4.012 / 3.988 |
| 8192² | older_read_at | 8 | 116.986 | 116.972 | 81.277 | 2.425 | 4.012 / 3.988 |
| 8192² | next_read_at | 8 | 116.374 | 116.347 | 79.665 | 2.000 | 4.012 / 3.988 |
| 8192² | older_coroutine | 4 | 129.352 | 129.321 | 81.538 | 1.900 | 4.012 / 3.988 |
| 8192² | older_tokio_inline | 8 | 117.559 | 117.542 | 82.837 | 1.575 | 4.012 / 3.988 |
| 8192² | next_tokio_inline | 8 | 118.400 | 118.369 | 80.863 | 2.800 | 4.012 / 3.988 |
| 8192² | older_tokio_pool | 8 | 119.049 | 118.964 | 81.660 | 262.750 | 4.012 / 3.988 |
| 8192² | next_tokio_pool | 8 | 121.803 | 121.683 | 85.802 | 260.800 | 4.012 / 3.988 |
| 8192² | next_buffered_one_hop | 8 | 118.922 | 118.869 | 81.069 | 4.550 | 4.012 / 3.988 |
| 8192² | next_tokio_in_place | 8 | 117.206 | 117.174 | 79.603 | 1.275 | 4.012 / 3.988 |
| 8192² | next_streamed_inline | 8 | 117.859 | 117.820 | 80.191 | 2.775 | 4.012 / 3.988 |
| 8192² | next_streamed_pool | 4 | 118.969 | 118.821 | 86.421 | 260.750 | 4.012 / 3.988 |
| 8192² | next_streamed_in_place | 8 | 115.383 | 115.357 | 78.353 | 1.600 | 4.012 / 3.988 |
| 8192² | next_seek_read | 8 | 118.101 | 118.058 | 80.967 | 3.375 | 4.012 / 3.988 |

## Native resident-path contrasts

Differences in microseconds/decode; A/A spread shown alongside each. Negative means the first named path used less time. An unresolved result is not a win or a tie.

| Codec / size | Contrast | Wall delta / A/A | Wall resolved? | CPU delta / A/A | CPU resolved? |
|---|---|---:|---|---:|---|
| qoi 4096² | next_slice − native_probed | -14.604 / 1892.484 | unresolved | -17.988 / 1886.125 | unresolved |
| qoi 4096² | borrowed_dyn − native | -151.694 / 1892.484 | unresolved | -148.750 / 1886.125 | unresolved |
| qoi 4096² | native_probed − native | -308.077 / 1892.484 | unresolved | -304.775 / 1886.125 | unresolved |
| qoi 8192² | next_slice − native_probed | 779.254 / 2968.701 | unresolved | 728.400 / 2943.250 | unresolved |
| qoi 8192² | borrowed_dyn − native | 1841.417 / 2968.701 | unresolved | 1810.750 / 2943.250 | unresolved |
| qoi 8192² | native_probed − native | -91.474 / 2968.701 | unresolved | -112.800 / 2943.250 | unresolved |
| bmp 4096² | next_slice − native_probed | 493.884 / 429.877 | yes | 491.725 / 422.000 | yes |
| bmp 4096² | borrowed_dyn − native | 293.477 / 429.877 | unresolved | 289.819 / 422.000 | unresolved |
| bmp 4096² | native_probed − native | -235.221 / 429.877 | unresolved | -233.750 / 422.000 | unresolved |
| bmp 8192² | next_slice − native_probed | 194.087 / 3274.072 | unresolved | 186.025 / 3297.875 | unresolved |
| bmp 8192² | borrowed_dyn − native | 688.620 / 3274.072 | unresolved | 667.675 / 3297.875 | unresolved |
| bmp 8192² | native_probed − native | -178.071 / 3274.072 | unresolved | -182.900 / 3297.875 | unresolved |
| jpeg 4096² | next_slice − native_probed | 106.628 / 2721.967 | unresolved | 88.525 / 2749.000 | unresolved |
| jpeg 4096² | borrowed_dyn − native | 622.870 / 2721.967 | unresolved | 637.125 / 2749.000 | unresolved |
| jpeg 4096² | native_probed − native | 379.289 / 2721.967 | unresolved | 389.525 / 2749.000 | unresolved |
| jpeg 8192² | next_slice − native_probed | -1454.029 / 4501.515 | unresolved | -1555.400 / 4512.000 | unresolved |
| jpeg 8192² | borrowed_dyn − native | 893.892 / 4501.515 | unresolved | 845.000 / 4512.000 | unresolved |
| jpeg 8192² | native_probed − native | 3156.475 / 4501.515 | unresolved | 3134.900 / 4512.000 | unresolved |

## Identity and qualification

Source: `2a7849d97d4ddeda8ab6e1a03f5180881993e1e8`. All 670 pairs retained; source hashes unchanged. Minimum measured adjusted loop: 463.306 ms. Calibration targeted at least 500 ms before planning; later block durations were not used to select or discard cells.

Native sources: zenbitmaps `9d71b356f290f6438b810cc53d99f29301f6d247`, zenjpeg `6d6539dfaf3b9e09d74ad9ef7a56b4e4e7518dd0`. The two benchmark-only bitmap accessors are source-verified; native kernels are unchanged. Tracked dependency locks preserve both graphs. Full source hashes, exact process commands, fixture hashes, PIDs and raw counters are retained with the run.

Before timing, the original 8K BMP prefix probe failed: it unwrapped a native amplification refusal on a 64 KiB prefix. That attempt contains zero calibration samples and zero timing pairs. It is preserved in the adjacent `large-study/` directory. The corrected valid-file binding asks for more bytes and retains a 128-byte regression. This is not general malformed-file classification qualification.

The inherited async planner also continued source reads after a short read signalled cancellation. The failing-first control observed 22 reads; the repaired decode/transcode planning loops check optional Pulse between reads, and the control requires exactly one. Native errors retain priority over a stop raised by their failing call.

Local standalone-workspace tests on r7900x, strict Clippy, strict Rustdoc, formatting and genuine no_std WASM/freestanding builds pass for the optional-Pulse source. Native/synthetic probe suites and strict Clippy pass with the corrected binding. The parent-repository CI and Fuzz runs pass (they do not test this separate workspace); the four native documentation qualification jobs still fail on the pre-existing archmage version conflict. They remain unqualified.

Native probe SHA256: `7eeaef3a65586a5bfe689da90120c8f3d05872f66662598a52cde73d2b1f95b3`.

Synthetic probe SHA256: `5f015dcfa2c0e37d160974b12ad5957f5ffbb0a124edcdbae0f790fdbb8ac825`.

Raw artifact directory: `/mnt/v/output/zencodec/reimagined-optional-pulse-2026-11-12/large-study-repaired`. The compact JSON receipt beside this report uses microseconds. This report’s main tables use milliseconds.

Reproduction: `PROBE_TARGET=... TMPDIR=$HOME/tmp run-heavy --mem 4G --jobs 2 -- just -f reimagined/justfile probe-check`; build with `probe-build`, then set `ZENBENCH_EXECUTOR` to the retained verified zenbench performance executor and run `large-profile FRESH_OUTPUT` under the same wrapper. `probe-setup` prepares fresh exports and locks. No native CPU flags.

Toolchain: Rust 1.99.1 / LLVM 23.1.1; r7900x (Ryzen 9 7900X), verified uniform performance-class cpuset 0–23. Two runtime workers and explicit build/pool limits; no native CPU flags.

`probe-build-first.log`: `run-heavy: done rc=101 77s | peak-RSS 0.65GiB | min-avail 12306MiB | peak-load 1.71`

`probe-build-second.log`: `run-heavy: done rc=101 41s | peak-RSS 0.45GiB | min-avail 12648MiB | peak-load 1.51`

`probe-build-third.log`: `run-heavy: done rc=0 22s | peak-RSS 0.25GiB | min-avail 12947MiB | peak-load 1.38`

`probe-build-fourth.log`: `run-heavy: done rc=0 28s | peak-RSS 0.27GiB | min-avail 12935MiB | peak-load 1.31`

`large-study.log`: `run-heavy: done rc=1 16s | peak-RSS 0.88GiB | min-avail 12474MiB | peak-load 0.93`

`large-study-repaired.log`: `run-heavy: done rc=0 1411s | peak-RSS 0.88GiB | min-avail 12239MiB | peak-load 1.44`
