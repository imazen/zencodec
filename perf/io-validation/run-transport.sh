#!/usr/bin/env bash
# Run under the caller's resource wrapper in an exported source tree.
# The two input directories come from the pinned containers' FFmpeg oracle scripts.
set -euo pipefail
mp4=$(realpath "$1")
webm=$(realpath "$2")
out=$(realpath -m "$3")
test ! -e "$out"
mkdir -p "$out"
ffmpeg -v error -i "$mp4/av1_opus_faststart.mp4" -map 0:v:0 -c copy -f ivf "$out/av1.ivf"
ffmpeg -v error -i "$webm/vp9_opus.webm" -map 0:v:0 -c copy -f ivf "$out/vp9.ivf"
cargo run --locked --manifest-path perf/io-validation/Cargo.toml --bin media_transport -- \
    "$mp4"/*.mp4 "$webm"/*.webm "$webm"/*.mkv "$out"/*.ivf
