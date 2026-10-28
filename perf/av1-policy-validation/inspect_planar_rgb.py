#!/usr/bin/env python3
"""Compare retained native RGB to FFmpeg/zimg without inventing a pass threshold.

The native test creates all source AV1 and zen RGB. The independent converter
only inspects them. Documentation: https://ffmpeg.org/ffmpeg-filters.html#zscale
This reports disagreement and clipping; it is not an RGB correctness gate.
"""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path
import shutil
import struct
import subprocess

# Absolute normalized RGB error for this bounded BT.709/sRGB matrix route.
# This allows float arithmetic variation, not integer quantization, clipping,
# transfer conversion or an arbitrary CMS. Keep it fixed across oracle runs.
FLOAT_ABS_LIMIT = 1e-6


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def extend_for_zimg(source, info):
    """zimg requires multiples of subsampling. Extend edges, never rescale.

    Luma grows to an even extent; ceil-sized chroma remains the same size.
    Comparison later selects only the original rectangle. This preserves the
    native kernels' edge-replication convention without changing any input code.
    """
    sx = 1 if info["subsampling"] == "Yuv444" else 2
    sy = 2 if info["subsampling"] == "Yuv420" else 1
    width, height = info["width"], info["height"]
    padded_width = (width + sx - 1) // sx * sx
    padded_height = (height + sy - 1) // sy * sy
    word = 1 if info["bits"] == 8 else 2
    output = bytearray()
    offset = 0
    for _ in range(info["frames"]):
        for dx, dy in [(1, 1), (sx, sy), (sx, sy)]:
            w, h = (width + dx - 1) // dx, (height + dy - 1) // dy
            pw, ph = padded_width // dx, padded_height // dy
            plane = source[offset:offset + w * h * word]
            if len(plane) != w * h * word:
                raise ValueError("independent native sample data is truncated")
            for y in range(ph):
                row = plane[min(y, h - 1) * w * word: (min(y, h - 1) + 1) * w * word]
                expanded = row + row[-word:] * (pw - w)
                if expanded[:w * word] != row:
                    raise ValueError("edge extension changed an original source sample")
                output.extend(expanded)
            offset += len(plane)
    if offset != len(source):
        raise ValueError("independent native sample data has trailing bytes")
    return bytes(output), padded_width, padded_height


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--float-output", action="store_true", help="require and inspect native unclipped RGB F32 output too")
    parser.add_argument("--verify-float", action="store_true", help="enforce the fixed 1e-6 absolute float agreement gate")
    args = parser.parse_args()
    if args.verify_float and not args.float_output:
        parser.error("--verify-float requires --float-output")
    args.output.mkdir(parents=True, exist_ok=False)
    ffmpeg = Path(shutil.which("ffmpeg") or "ffmpeg")
    version = subprocess.run([str(ffmpeg), "-version"], check=True, capture_output=True, text=True, timeout=10)
    (args.output / "ffmpeg-version.txt").write_text(version.stdout)
    manifest = {"ffmpeg": str(ffmpeg), "sha256": digest(ffmpeg), "cells": [],
                "meaning": "Fixed absolute float agreement gate; integer differences are observations, not accepted accuracy" if args.verify_float else "Observed disagreement; no acceptance threshold and no product timing claim",
                "docs": "https://ffmpeg.org/ffmpeg-filters.html#zscale"}
    results = []
    layouts = sorted(args.source.glob("planar-*/rgb-layout.json"))
    if len(layouts) != 18:
        raise ValueError(f"expected all 18 native matrix cells, found {len(layouts)}")
    for layout in layouts:
        info = json.loads(layout.read_text())
        cell = args.output / layout.parent.name
        cell.mkdir()
        encoded = layout.parent / "approved.ivf"
        native_codes = layout.parent / "dav1d.raw"
        extended_codes = cell / "edge-extended-yuv.raw"
        extended, padded_width, padded_height = extend_for_zimg(native_codes.read_bytes(), info)
        extended_codes.write_bytes(extended)
        yuv = {"Yuv420": "yuv420p", "Yuv422": "yuv422p", "Yuv444": "yuv444p"}[info["subsampling"]]
        if info["bits"] != 8:
            yuv += str(info["bits"]) + "le"
        converted = cell / "zimg-gbrpf32le.raw"
        # Explicit interpretation prevents container/default precedence from
        # changing the operation under comparison. No transfer/gamut conversion.
        filter_graph = ("zscale=matrixin=709:matrix=0:primariesin=709:primaries=709:"
                        "transferin=iec61966-2-1:transfer=iec61966-2-1:"
                        f"rangein={'full' if info['full_range'] else 'limited'}:range=full:"
                        "chromalin=center:filter=bilinear:dither=none,format=gbrpf32le")
        command = [str(ffmpeg), "-v", "error", "-nostdin", "-threads", "1", "-filter_threads", "1",
                   "-f", "rawvideo", "-pixel_format", yuv, "-video_size", f"{padded_width}x{padded_height}",
                   "-framerate", "25", "-i", str(extended_codes), "-vf", filter_graph, "-fps_mode", "passthrough",
                   "-f", "rawvideo", "-pix_fmt", "gbrpf32le", str(converted)]
        (cell / "command.json").write_text(json.dumps(command, indent=2) + "\n")
        run = subprocess.run(command, capture_output=True, timeout=30)
        (cell / "stderr.txt").write_bytes(run.stderr)
        run.check_returncode()
        count = info["width"] * info["height"]
        padded_count = padded_width * padded_height
        raw = converted.read_bytes()
        expected_size = info["frames"] * padded_count * 3 * 4
        if len(raw) != expected_size:
            raise ValueError(f"{cell.name}: expected {expected_size} bytes, got {len(raw)}")
        floats = struct.unpack(f"<{len(raw)//4}f", raw)
        native_hashes = {}
        for frame in range(info["frames"]):
            native = layout.parent / f"zen-rgb16-{frame}.raw"
            data = native.read_bytes()
            if len(data) != count * 6:
                raise ValueError(f"{native}: unexpected output extent")
            native_float = layout.parent / f"zen-rgbf32-{frame}.raw"
            float_words = None
            if args.float_output:
                float_data = native_float.read_bytes()
                if len(float_data) != count * 12:
                    raise ValueError(f"{native_float}: unexpected float output extent")
                float_words = struct.unpack(f"<{count*3}f", float_data)
            words = struct.unpack(f"<{count*3}H", data)
            rgb = floats[frame * padded_count * 3:(frame + 1) * padded_count * 3]
            differences, extended, float_differences, clamped_float_differences = [], [], [], []
            for pixel in range(count):
                for channel, plane in enumerate([2, 0, 1]):  # planar G,B,R -> packed R,G,B
                    y, x = divmod(pixel, info["width"])
                    value = rgb[plane * padded_count + y * padded_width + x]
                    if not math.isfinite(value):
                        raise ValueError(f"{cell.name}: non-finite independent result")
                    extended.append(value)
                    if float_words is not None:
                        actual_float = float_words[pixel * 3 + channel]
                        if not math.isfinite(actual_float):
                            raise ValueError(f"{native_float}: non-finite native result")
                        float_differences.append(abs(actual_float - value))
                        clamped_float_differences.append(abs(min(1.0, max(0.0, actual_float)) - value))
                    differences.append(abs(words[pixel * 3 + channel] / 65535 - min(1.0, max(0.0, value))))
            results.append({**info, "frame": frame, "samples": len(differences),
                            "below_zero": sum(v < 0 for v in extended), "above_one": sum(v > 1 for v in extended),
                            "minimum": min(extended), "maximum": max(extended),
                            "max_abs_after_clipping": max(differences),
                            "mean_abs_after_clipping": sum(differences) / len(differences)})
            if float_words is not None:
                results[-1].update(max_abs_float=max(float_differences),
                                  mean_abs_float=sum(float_differences) / len(float_differences),
                                  max_abs_if_native_clipped=max(clamped_float_differences),
                                  native_float_min=min(float_words), native_float_max=max(float_words))
                native_hashes[native_float.name] = digest(native_float)
            native_hashes[native.name] = digest(native)
        manifest["cells"].append({"name": cell.name, "source_sha256": digest(encoded),
                                  "independent_native_sha256": digest(native_codes),
                                  "edge_extended_sha256": digest(extended_codes),
                                  "edge_extended_size": [padded_width, padded_height],
                                  "layout_sha256": digest(layout), "native_rgb_sha256": native_hashes,
                                  "independent_rgb_sha256": digest(converted)})
        # Retain completed cells even if a later oracle fails.
        (args.output / "provenance.json").write_text(json.dumps(manifest, indent=2) + "\n")
        with (args.output / "comparison.csv").open("w", newline="") as stream:
            writer = csv.DictWriter(stream, fieldnames=list(results[0]), lineterminator="\n")
            writer.writeheader()
            writer.writerows(results)
    print(f"inspected {len(results)} presentations; evidence: {args.output / 'comparison.csv'}")
    if args.verify_float:
        rejected = [r for r in results if r["max_abs_float"] > FLOAT_ABS_LIMIT]
        missed_clipping = [r for r in results if r["max_abs_if_native_clipped"] <= FLOAT_ABS_LIMIT]
        verdict = {"absolute_limit": FLOAT_ABS_LIMIT, "presentations": len(results),
                   "outside_tolerance": len(rejected), "clipping_controls_not_detected": len(missed_clipping)}
        (args.output / "float-verification.json").write_text(json.dumps(verdict, indent=2) + "\n")
        if rejected or missed_clipping:
            raise ValueError(f"float verification failed: {verdict}")
        print(f"float agreement and deliberate-clipping controls passed for {len(results)} presentations")


if __name__ == "__main__":
    main()
