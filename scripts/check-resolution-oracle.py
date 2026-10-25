#!/usr/bin/env python3
"""Compare the public density helpers with Python's exact rational arithmetic."""
import argparse
from fractions import Fraction
import itertools
import math
import random
import struct
import subprocess


FACTORS = {"I": Fraction(5000, 127), "C": Fraction(100), "M": Fraction(1)}
UNITS = "ICMU"
LIMITS = (0, 1, 65535, 2**31 - 1, 2**32 - 1)


def bits(value):
    return struct.unpack(">Q", struct.pack(">d", value))[0]


def value(raw):
    return struct.unpack(">d", struct.pack(">Q", raw))[0]


def valid(pair):
    return all(math.isfinite(v) and v > 0 for v in pair)


def equivalent(a, au, b, bu):
    if not valid(a) or not valid(b):
        return False
    a, b = tuple(map(Fraction, a)), tuple(map(Fraction, b))
    if au == bu == "U":
        return a[0] / a[1] == b[0] / b[1]
    if "U" in (au, bu):
        return False
    return all(x * FACTORS[au] == y * FACTORS[bu] for x, y in zip(a, b))


def integer(a, au, target, maximum):
    if not valid(a):
        return "InvalidValue"
    if (au == "U") != (target == "U"):
        return "UnitMismatch"
    if au == "U":
        ratio = Fraction(a[0]) / Fraction(a[1])
        result = ratio.numerator, ratio.denominator
    else:
        exact = [Fraction(v) * FACTORS[au] / FACTORS[target] for v in a]
        if any(v.denominator != 1 for v in exact):
            return "NotRepresentable"
        result = tuple(v.numerator for v in exact)
    if not all(0 < v <= maximum for v in result):
        return "NotRepresentable"
    return " ".join(map(str, result))


def cases():
    special = [
        0.0, -0.0, -1.0, math.inf, -math.inf, math.nan,
        value(1), value(2), value(2**52 - 1), value(2**52),
        value(0x7FEFFFFFFFFFFFFF), 0.25, 0.5, 1.0, 2.0,
        127.0, 254.0, 300.0, 3780.0, 5000.0, 10000.0,
        65535.0, float(2**31 - 1), float(2**32 - 1), float(2**32),
    ]
    special += [math.nextafter(v, direction) for v in special[11:] for direction in (0.0, math.inf)]
    for x, au, target, maximum in itertools.product(special, UNITS, UNITS, LIMITS):
        # Both axis positions, identity, inverse aspect, and a neighbouring value.
        for a, b in [((x, 1.0), (x, 1.0)), ((1.0, x), (x, 1.0)),
                     ((x, x), (1.0, 1.0)), ((x, 1.0), (math.nextafter(x, math.inf), 1.0))]:
            yield a, au, b, target, target, maximum
    for scale in (1, 2, 3, 127, 65535):
        for au, bu in itertools.product(UNITS, UNITS):
            a = (float(127 * scale), float(254 * scale))
            b = tuple(float(Fraction(v) * FACTORS.get(au, 1) / FACTORS.get(bu, 1)) for v in a)
            yield a, au, b, bu, bu, 2**32 - 1
    rng = random.Random(0xD3A517)
    for _ in range(12000):
        a = value(rng.getrandbits(64)), value(rng.getrandbits(64))
        b = value(rng.getrandbits(64)), value(rng.getrandbits(64))
        yield a, rng.choice(UNITS), b, rng.choice(UNITS), rng.choice(UNITS), rng.choice(LIMITS)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("probe")
    args = parser.parse_args()
    inputs, expected = [], []
    for a, au, b, bu, target, maximum in cases():
        inputs.append(f"{bits(a[0]):016x} {bits(a[1]):016x} {au} {bits(b[0]):016x} {bits(b[1]):016x} {bu} {target} {maximum}")
        expected.append(f"{str(equivalent(a, au, b, bu)).lower()} {integer(a, au, target, maximum)}")
    result = subprocess.run([args.probe], input="\n".join(inputs) + "\n", text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30, check=True)
    actual = result.stdout.splitlines()
    assert len(actual) == len(expected), (len(actual), len(expected), result.stderr)
    for request, wanted, got in zip(inputs, expected, actual):
        assert wanted == got, f"request={request}\nFraction={wanted}\nzencodec={got}"
    print(f"Fraction oracle: {len(expected)} exact equivalence and integer-carriage cases passed")


if __name__ == "__main__":
    main()
