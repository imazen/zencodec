# Decode path: `Vec<u8>` storage against `PixelStorage` (2026-11-04)

Gate for adopting `PixelStorage` in `zencodec`. `zencodec-io/benches/decode_storage.rs` runs the
pixel paths a decode takes: rows into `CollectSink`, reading the result, and `to_owned_frame`.
A decoder is simulated by filling each strip. **No real codec ran here.**

- **Host:** r7900x (Zen 4, Ryzen 9 7900X), rustc 1.99.1, load under 2.3. Not measured on Zen 5.
- **old:** zencodec `993b9a04` with zenpixels `0d369697`, the commit before `PixelStorage`, renaming
  only `PlaneStorage` to `SampleStorage`.
- **new:** this commit, with zenpixels `85c0c104`.
- **Method:** one binary per side, run alternately three times with `--no-busy-gate`, 20 fixed rounds
  per group through zenbench. `old` and `new` are medians of the three per-run means, and `per-run`
  is each pair's ratio. Readings at the timer's resolution print as `0ns` or `nan`.

```text
case                  bench                  old       new  new/old  per-run
u8 RGB8 64x64         collect              358ns     367ns    1.025  1.057 0.956 1.017
u8 RGB8 64x64         read rows             19ns      21ns    1.105  1.105 1.105 1.050
u8 RGB8 64x64         to_owned_frame       179ns     219ns    1.223  1.443 0.973 1.168
u8 RGB8 1024x1024     collect             47.7us    45.7us    0.958  0.981 0.959 0.958
u8 RGB8 1024x1024     read rows            600ns     600ns    1.000  1.600 1.000 0.833
u8 RGB8 1024x1024     to_owned_frame      46.8us    47.8us    1.021  1.026 1.023 1.017
u8 RGB8 4096x4096     collect            16.60ms   16.60ms    1.000  0.976 1.024 1.000
u8 RGB8 4096x4096     read rows              0ns       0ns      nan  
u8 RGB8 4096x4096     to_owned_frame     18.40ms   18.40ms    1.000  0.989 1.016 1.000
u16 RGB16 64x64       collect              502ns     563ns    1.122  1.022 1.139 1.009
u16 RGB16 64x64       read rows             19ns      20ns    1.053  1.053 1.105 1.053
u16 RGB16 64x64       to_owned_frame       353ns     424ns    1.201  1.052 1.201 1.017
u16 RGB16 1024x1024   collect             89.1us    88.4us    0.992  0.964 1.069 0.992
u16 RGB16 1024x1024   read rows            1.7us     1.6us    0.941  1.000 0.941 0.941
u16 RGB16 1024x1024   to_owned_frame     100.7us   100.6us    0.999  0.947 1.072 0.999
u16 RGB16 4096x4096   collect            35.30ms   35.10ms    0.994  0.989 0.981 0.994
u16 RGB16 4096x4096   read rows              0ns       0ns      nan  
u16 RGB16 4096x4096   to_owned_frame     37.90ms   37.60ms    0.992  0.987 0.977 1.000
f32 RGBA 64x64        collect              1.2us     1.0us    0.833  0.799 1.000 0.909
f32 RGBA 64x64        read rows              0ns       0ns      nan  
f32 RGBA 64x64        to_owned_frame       800ns     873ns    1.091  0.970 1.000 1.125
f32 RGBA 1024x1024    collect            271.1us   274.5us    1.013  0.977 1.025 1.032
f32 RGBA 1024x1024    read rows            4.0us     4.0us    1.000  0.784 1.025 0.975
f32 RGBA 1024x1024    to_owned_frame     494.5us   490.3us    0.992  0.993 0.979 0.975
f32 RGBA 4096x4096    collect            96.30ms   94.40ms    0.980  0.997 0.976 0.981
f32 RGBA 4096x4096    read rows          100.0us   100.0us    1.000  1.000 1.000 1.000
f32 RGBA 4096x4096    to_owned_frame     99.20ms   98.00ms    0.988  0.968 0.988 0.974
```

**Reading it**

- **1024x1024 and 4096x4096:** within 4% on median in every readable case, with collect and
  `to_owned_frame` ratios from 0.958 to 1.021. Per-run ratios spread more, from 0.947 to 1.072.
- **64x64:** `to_owned_frame` costs 40 to 73 ns more on median and `read rows` 1 to 2 ns more.
  `collect` ranges from 0.83x to 1.12x. These runs are noisy: `to_owned_frame` at RGB8 ranged from
  0.97x to 1.44x between runs.

The first version of this change, which zeroed the allocation before copying, measured
`to_owned_frame` at 2.2x for RGB8 64x64 and 1.4x at 1024x1024. The non-zeroing copy fixed that.
