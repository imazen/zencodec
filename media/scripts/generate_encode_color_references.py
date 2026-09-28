#!/usr/bin/env python3
"""Decimal RGB16 → native integer codes, independent of production kernels."""
from decimal import Decimal as D, ROUND_HALF_UP, localcontext
from itertools import product
from pathlib import Path
import argparse
import json

ROOT = Path(__file__).resolve().parents[1]


def generate():
    result = []
    samples = list(product([0, 65535], repeat=3)) + [(32768,32768,32768),(12345,45678,54321),(1,65534,513)]
    with localcontext() as ctx:
        ctx.prec = 70
        for bits, full, matrix, sample in product([8,10,12,16],[False,True],[0,1,6,9],samples):
            r,g,b = [D(v)/65535 for v in sample]
            k, maximum, neutral = 2**(bits-8),2**bits-1,2**(bits-1)
            if matrix == 0:
                values = [g,b,r]
            else:
                kr,kb = {1:(D('.2126'),D('.0722')),6:(D('.299'),D('.114')),9:(D('.2627'),D('.0593'))}[matrix]
                y = kr*r+(1-kr-kb)*g+kb*b
                values = [y,(b-y)/(2*(1-kb)),(r-y)/(2*(1-kr))]
            codes=[]
            for i, v in enumerate(values):
                if i == 0 or matrix == 0:
                    code = v*maximum if full else 16*k+v*219*k
                else:
                    code = neutral+v*(maximum if full else 224*k)
                codes.append(int(min(D(maximum),max(D(0),code)).to_integral_value(rounding=ROUND_HALF_UP)))
            result.append(dict(bits=bits,full=full,matrix=matrix,rgb16=sample,codes=codes))
    return json.dumps(result,indent=2)+'\n'


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args()
    path=ROOT/'corpus/encode-color-references.json'
    data=generate()
    if args.check:
        if path.read_text()!=data:
            raise SystemExit('encode-color references differ; regenerate and review')
    else:
        path.write_text(data)
    print(f'{path.name}: {len(json.loads(data))} independently quantized vectors',flush=True)


if __name__=='__main__':
    main()
