# zencodec cold build time: crates.io 0.1.26 vs main, 2026-10-15

`scripts/cold-build-time.sh ecca0caa1823925a725f0b53f79db9c6de0f9897 5` on dev (Ryzen 9 9950X3D,
32 threads, default cargo parallelism, `nice -n19`). Four throwaway crates depend on zencodec
`=0.1.26` (crates.io) or main `ecca0caa`, each with and without `std`. Dependencies are fetched
first; every build starts from an empty `target/`; variants alternate, 5 runs each. The zencodec
column is that crate's own unit time from `cargo build --timings`. An SWE-2 optimization lane was
running on dev at the same time, so ranges are wide; the medians move together.

| variant | profile | wall, median (range) | zencodec unit, median (range) |
|---|---|---|---|
| 0.1.26 | dev | 1.84 s (1.49-2.15) | 0.40 s (0.31-0.45) |
| main | dev | 2.06 s (1.70-2.43) | 0.65 s (0.52-0.86) |
| 0.1.26 + std | dev | 1.77 s (1.74-2.14) | 0.38 s (0.34-0.45) |
| main + std | dev | 2.13 s (2.08-2.42) | 0.63 s (0.61-0.74) |
| 0.1.26 | release | 2.01 s (1.93-2.08) | 0.50 s (0.48-0.56) |
| main | release | 2.28 s (2.10-2.54) | 0.81 s (0.75-0.91) |
| 0.1.26 + std | release | 1.91 s (1.72-2.17) | 0.50 s (0.46-0.55) |
| main + std | release | 2.22 s (1.89-2.68) | 0.82 s (0.68-0.96) |

Main adds 0.25 s (dev) to 0.31 s (release) to zencodec's own compile, and 0.22-0.27 s of wall
time for zencodec and its dependencies. Source grew from 13,583 to 18,697 non-comment lines; about
1,500 of the added lines are the unpublished streaming and sans-I/O surface (`traits/incremental.rs`,
`source.rs`, `output_buffer.rs`, `helpers/whole_file.rs`, the `begin`/`pending_output`/`take_output`
defaults); most of the rest is metadata (`inventory.rs`, `set.rs`, `exif.rs`,
`metadata_rewrite.rs`). A cargo feature gating the streaming surface could therefore save at most
the measured 0.25-0.31 s; the gate itself was not built or measured.
