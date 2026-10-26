#!/usr/bin/env python3
"""Evaluate a recorded worst sample directly from matrix/type-3 ICC tags.

Diagnostic only, not a general ICC parser or another product color converter.
Models: https://archive.color.org/specification/ICC.1-2022-05.pdf
sections 10.18 (parametricCurveType) and F.3 (matrix-based profiles).
"""
import json
from pathlib import Path
import struct
import sys


def profile(path):
    data = path.read_bytes()
    assert data[16:24] == b"RGB XYZ "
    count = struct.unpack_from(">I", data, 128)[0]
    assert count <= 64
    tags = {}
    for i in range(count):
        name, offset, size = struct.unpack_from(">4sII", data, 132 + i * 12)
        assert offset + size <= len(data)
        tags[name] = data[offset:offset + size]
    assert not any(name in tags for name in [b"A2B0", b"A2B1", b"B2A0", b"B2A1"])
    columns, curves = [], []
    for color in [b"r", b"g", b"b"]:
        xyz, curve = tags[color + b"XYZ"], tags[color + b"TRC"]
        assert xyz[:4] == b"XYZ " and len(xyz) == 20
        assert curve[:4] == b"para" and curve[8:10] == b"\0\3" and len(curve) == 32
        columns.append([x / 65536 for x in struct.unpack_from(">3i", xyz, 8)])
        curves.append([x / 65536 for x in struct.unpack_from(">5i", curve, 12)])
    return list(map(list, zip(*columns))), curves


def decode(value, curve):
    gamma, a, b, c, d = curve
    value = min(1.0, max(0.0, value))
    return (a * value + b) ** gamma if value >= d else c * value


def encode(value, curve):
    gamma, a, b, c, d = curve
    value = min(1.0, max(0.0, value))
    return (value ** (1 / gamma) - b) / a if value >= (a * d + b) ** gamma else value / c


def solve(matrix, right):
    rows = [row[:] + [value] for row, value in zip(matrix, right)]
    for col in range(3):
        pivot = max(range(col, 3), key=lambda i: abs(rows[i][col]))
        rows[col], rows[pivot] = rows[pivot], rows[col]
        divisor = rows[col][col]
        assert abs(divisor) > 1e-12
        rows[col] = [value / divisor for value in rows[col]]
        for i in range(3):
            if i != col:
                factor = rows[i][col]
                rows[i] = [a - factor * b for a, b in zip(rows[i], rows[col])]
    return [row[3] for row in rows]


def main():
    directory = Path(sys.argv[1])
    report = json.loads((directory / "comparison.json").read_text())
    row, sample, *_ = report["worst"]
    channels = report["channels"]
    pixel = sample // channels
    offset = (row * report["width"] + pixel) * channels * 4
    values = {}
    for filename in ["source.f32", "converted.f32", "oracle.f32"]:
        with (directory / filename).open("rb") as file:
            file.seek(offset)
            values[filename] = struct.unpack("=3f", file.read(12))
    source, source_curves = profile(directory / "source.icc")
    target, target_curves = profile(directory / "target.icc")
    linear = [decode(v, c) for v, c in zip(values["source.f32"], source_curves)]
    xyz = [sum(a * b for a, b in zip(row, linear)) for row in source]
    result = [encode(v, c) for v, c in zip(solve(target, xyz), target_curves)]
    values["direct_f64_matrix_and_curves"] = result
    values["position"] = [pixel, row]
    values["moxcms_error"] = [a - b for a, b in zip(values["converted.f32"], result)]
    values["lcms_error"] = [a - b for a, b in zip(values["oracle.f32"], result)]
    output = directory / "direct-math.json"
    with output.open("x") as file:
        file.write(json.dumps(values, indent=2) + "\n")
    print(json.dumps(values))


if __name__ == "__main__":
    main()
