#!/usr/bin/env python3
"""Independent Decimal display and primary references, with no codec imports."""
from decimal import Decimal as D, localcontext
from itertools import product
from pathlib import Path
import argparse
import json

ROOT = Path(__file__).resolve().parents[1]


def inv(matrix):
    """Gauss-Jordan elimination, independent of Rust's closed-form inverse."""
    a = [list(row) + [D(i == j) for j in range(3)] for i, row in enumerate(matrix)]
    for col in range(3):
        pivot = max(range(col, 3), key=lambda row: abs(a[row][col]))
        a[col], a[pivot] = a[pivot], a[col]
        scale = a[col][col]
        a[col] = [v / scale for v in a[col]]
        for row in range(3):
            if row != col:
                factor = a[row][col]
                a[row] = [v-factor*p for v, p in zip(a[row], a[col])]
    return [row[3:] for row in a]


def dot(matrix, vector):
    return [sum(a*b for a, b in zip(row, vector)) for row in matrix]


def xyz(primaries):
    points = {1: ['.640', '.330', '.300', '.600', '.150', '.060'],
              9: ['.708', '.292', '.170', '.797', '.131', '.046'],
              12: ['.680', '.320', '.265', '.690', '.150', '.060']}[primaries]
    r, ry, g, gy, b, by = map(D, points)
    columns = [[r/ry, D(1), (1-r-ry)/ry],
               [g/gy, D(1), (1-g-gy)/gy],
               [b/by, D(1), (1-b-by)/by]]
    matrix = list(map(list, zip(*columns)))
    wx, wy = D('.3127'), D('.3290')
    scales = dot(inv(matrix), [wx/wy, D(1), (1-wx-wy)/wy])
    return [[v*s for v, s in zip(row, scales)] for row in matrix]


def decode(curve, rgb, peak, black, gamma):
    if curve == 'srgb':
        return [peak * (v/D('12.92') if v <= D('.04045') else
                        ((v+D('.055'))/D('1.055'))**D('2.4')) for v in rgb]
    if curve == 'pq':
        p = [v**(D(32)/2523) for v in rgb]
        return [D(10000) * (max(v-D(3424)/4096, D(0)) /
                            (D(2413)/128-D(2392)/128*v))**(D(16384)/2610) for v in p]
    if curve == 'bt1886':
        exponent = 1/D('2.4')
        a = (peak**exponent-black**exponent)**D('2.4')
        b = black**exponent/(peak**exponent-black**exponent)
        return [a*(v+b)**D('2.4') for v in rgb]
    if curve == 'linear':
        return [v*peak for v in rgb]
    assert curve == 'hlg'
    beta = (3*(black/peak)**(1/gamma)).sqrt()
    a = D('.17883277')
    b, c = 1-4*a, D('.5')-a*(4*a).ln()
    signal = [max(D(0), (1-beta)*v+beta) for v in rgb]
    scene = [v*v/3 if v <= D('.5') else (((v-c)/a).exp()+b)/12 for v in signal]
    y = sum(w*v for w, v in zip(map(D, ['.2627', '.6780', '.0593']), scene))
    if y == 0:
        return [D(0)]*3
    return [peak*y**(gamma-1)*v for v in scene]


def generate():
    result = {'precision_decimal_digits': 70, 'display': [], 'primaries': []}
    with localcontext() as ctx:
        ctx.prec = 70
        samples = [tuple(map(D, rgb)) for rgb in product(['0', '.18', '.75', '1'], repeat=3)]
        samples += [(D(x),)*3 for x in ['.0001', '.0031308', '.018', '.02', '.04045', '.25', '.5', '.9999']]
        for curve, peak, black, gamma in [('srgb', '100', '0', '1'), ('srgb', '203', '0', '1'),
                ('pq', '10000', '0', '1'), ('bt1886', '100', '0', '2.4'),
                ('bt1886', '100', '.1', '2.4'), ('linear', '203', '0', '1'),
                ('hlg', '1000', '0', '1.2'), ('hlg', '1000', '.005', '1.2'),
                ('hlg', '400', '.1', '1.033'), ('hlg', '4000', '0', '1.453')]:
            for rgb in samples:
                output = decode(curve, rgb, D(peak), D(black), D(gamma))
                result['display'].append(dict(curve=curve, peak=peak, black=black, gamma=gamma,
                                              signal=list(map(str, rgb)), nits=list(map(str, output))))
        for source, target in product([1, 9, 12], repeat=2):
            for rgb in samples[:64]:
                output = dot(inv(xyz(target)), dot(xyz(source), rgb))
                result['primaries'].append(dict(source=source, target=target,
                                                rgb=list(map(str, rgb)), converted=list(map(str, output))))
    return json.dumps(result, indent=2) + '\n'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    path = ROOT / 'corpus/display-references.json'
    data = generate()
    if args.check:
        if path.read_text() != data:
            raise SystemExit('display references differ; regenerate and review')
    else:
        path.write_text(data)
    parsed = json.loads(data)
    print(f'{path.name}: {len(parsed["display"])} display + {len(parsed["primaries"])} primary cases', flush=True)


if __name__ == '__main__':
    main()
