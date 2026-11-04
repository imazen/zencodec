# Decode path: `Vec<u8>` storage against `PixelStorage` (2026-11-04)

Gate for adopting `PixelStorage` in `zencodec`. `zencodec-io/benches/decode_storage.rs` runs the
pixel paths a decode takes: rows into `CollectSink`, reading the result, and `to_owned_frame`.
A decoder is simulated by filling each strip. **No real codec ran here.**

- **Host:** r7900x (Zen 4, Ryzen 9 7900X), rustc 1.99.1, load under 2.3. Not measured on Zen 5.
- **old:** zencodec `993b9a04` with zenpixels `0d369697`, the commit before `PixelStorage`, renaming
  only `PlaneStorage` to `SampleStorage`.
- **new:** this commit, with zenpixels `8de69899`, which zeroes through `alloc_zeroed`.
- **Method:** one binary per side, run alternately three times with `--no-busy-gate`, 20 fixed rounds
  per group through zenbench. `old` and `new` are medians of the three per-run means, and `per-run`
  is each pair's ratio. Readings at the timer's resolution print as `0ns` or `nan`.

```text
case                  bench                  old       new  new/old  per-run
u8 RGB8 64x64         collect              381ns     364ns    0.955  0.987 0.992 0.928
u8 RGB8 64x64         read rows             19ns      21ns    1.105  1.105 1.050 1.105
u8 RGB8 64x64         to_owned_frame       206ns     205ns    0.995  1.114 1.091 0.995
u8 RGB8 1024x1024     collect             47.3us    45.9us    0.970  1.000 0.946 0.975
u8 RGB8 1024x1024     read rows            600ns     600ns    1.000  1.000 1.000 1.000
u8 RGB8 1024x1024     to_owned_frame      47.7us    46.8us    0.981  1.000 0.900 1.002
u8 RGB8 4096x4096     collect            16.80ms   15.40ms    0.917  0.929 0.890 0.911
u8 RGB8 4096x4096     read rows              0ns       0ns      nan  
u8 RGB8 4096x4096     to_owned_frame     18.60ms   18.70ms    1.005  1.022 0.979 0.984
u16 RGB16 64x64       collect              563ns     502ns    0.892  1.016 0.877 1.018
u16 RGB16 64x64       read rows             19ns      21ns    1.105  1.053 1.105 1.105
u16 RGB16 64x64       to_owned_frame       407ns     390ns    0.958  1.127 0.889 1.074
u16 RGB16 1024x1024   collect             87.9us    87.5us    0.995  0.990 0.995 1.038
u16 RGB16 1024x1024   read rows            1.6us     1.6us    1.000  1.000 1.000 0.938
u16 RGB16 1024x1024   to_owned_frame      98.6us   101.2us    1.026  1.015 1.028 1.034
u16 RGB16 4096x4096   collect            36.10ms   31.60ms    0.875  0.886 0.845 0.897
u16 RGB16 4096x4096   read rows              0ns       0ns      nan  
u16 RGB16 4096x4096   to_owned_frame     38.40ms   37.80ms    0.984  0.992 0.959 0.987
f32 RGBA 64x64        collect              1.2us     1.2us    1.000  1.000 0.917 1.091
f32 RGBA 64x64        read rows              0ns       0ns      nan  
f32 RGBA 64x64        to_owned_frame       800ns     800ns    1.000  1.125 1.000 1.000
f32 RGBA 1024x1024    collect            262.3us   258.0us    0.984  0.981 1.017 1.003
f32 RGBA 1024x1024    read rows            3.9us     4.0us    1.026  1.026 1.000 1.026
f32 RGBA 1024x1024    to_owned_frame     497.6us   496.5us    0.998  0.994 1.001 1.004
f32 RGBA 4096x4096    collect           100.20ms   86.60ms    0.864  0.894 0.864 0.854
f32 RGBA 4096x4096    read rows          100.0us   100.0us    1.000  1.000 1.000 1.000
f32 RGBA 4096x4096    to_owned_frame    104.90ms  102.30ms    0.975  1.011 0.971 0.972
```

**Reading it**

- **4096x4096:** `collect` is 8 to 14% faster, because zeroing through `alloc_zeroed` skips the write
  of pages the decoder is about to fill. `to_owned_frame` is within 2.5%.
- **1024x1024:** within 3% on median in every readable case, with per-run ratios from 0.90 to 1.04.
- **64x64:** `collect` and `to_owned_frame` are within noise, with medians from 0.89x to 1.00x and
  per-run ratios from 0.88x to 1.13x. `read rows` costs 1 to 2 ns more.

The first version of this change, which zeroed the allocation and then copied, measured
`to_owned_frame` at 2.2x for RGB8 64x64 and 1.4x at 1024x1024. The non-zeroing copy fixed that.
