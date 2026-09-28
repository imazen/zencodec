#!/usr/bin/env python3
"""Generate independent high-precision conversion oracles (no codec imports)."""
from decimal import Decimal, localcontext
from pathlib import Path
import argparse
import csv
import io

ROOT = Path(__file__).resolve().parents[1]
D = Decimal


def vectors():
    out = io.StringIO(newline="")
    writer = csv.writer(out, lineterminator="\n")
    writer.writerow(["operation", "bits", "full", "matrix", "x", "y", "z", "r", "g", "b"])
    with localcontext() as ctx:
        ctx.prec = 70
        for bits in (8, 10, 12, 16):
            k, maximum, neutral = 2 ** (bits - 8), 2**bits - 1, 2 ** (bits - 1)
            for full in (False, True):
                black, white = (0, maximum) if full else (16*k, 235*k)
                samples = [(black, neutral, neutral), (white, neutral, neutral),
                           ((black+white)//2, neutral, neutral),
                           (black, 16*k, 240*k), (white, 240*k, 16*k),
                           (0, 0, maximum), (maximum, maximum, 0),
                           (min(white, 81*k), 90*k, 240*k)]
                for matrix, kr, kb in [(1, D("0.2126"), D("0.0722")),
                                       (6, D("0.299"), D("0.114")),
                                       (9, D("0.2627"), D("0.0593"))]:
                    for sample in samples:
                        y, cb, cr = map(D, sample)
                        y = (y-black)/(white-black)
                        span = maximum if full else 224*k
                        cb, cr = (cb-neutral)/span, (cr-neutral)/span
                        r, b = y + 2*(1-kr)*cr, y + 2*(1-kb)*cb
                        g = (y-kr*r-kb*b)/(1-kr-kb)
                        writer.writerow(["ycbcr", bits, int(full), matrix, *sample, r, g, b])
        for value in ("0", "0.0001", "0.0031308", "0.018", "0.04045", "0.18", "0.5", "0.75", "1"):
            x = D(value)
            srgb = x/D("12.92") if x <= D("0.04045") else ((x+D("0.055"))/D("1.055"))**D("2.4")
            power = x**(D(32)/D(2523))
            pq = D(10000)*(max(power-D(3424)/4096, D(0))/(D(2413)/128-D(2392)/128*power))**(D(16384)/2610)
            a = D("0.17883277")
            b, c = 1-4*a, D("0.5")-a*(4*a).ln()
            hlg = x*x/3 if x <= D("0.5") else (((x-c)/a).exp()+b)/12
            for op, expected in [("srgb", srgb), ("pq_nits", pq), ("hlg_scene", hlg)]:
                writer.writerow([op, "", "", "", value, "", "", expected, "", ""])
    return out.getvalue()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    destination, data = ROOT / "corpus/reference-vectors.csv", vectors()
    if args.check:
        if destination.read_text() != data:
            raise SystemExit("reference vectors differ; regenerate and review the numeric change")
    else:
        destination.write_text(data)
    print(f"{destination.name}: {len(data.splitlines())-1} high-precision cases")


if __name__ == "__main__":
    main()
