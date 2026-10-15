#!/usr/bin/env python3
"""Return-type shims so two codec main branches compile on zencodec 5ed2dade.

PR 149 changed `Encoder::begin` to return `OutputTiming` and
`AnimationFrameEncoder::push_frame_timed` to return the encoded duration.
zenbitmaps and IMIJ main implement the earlier unit-returning forms. The shim
keeps each body unchanged inside a closure and maps its `Ok(())`; the harness
never calls either method on the main side, so no measured path changes.
"""
import re
import sys


def wrap(path, fn, old_ret, new_ret, ok_map):
    src = open(path).read()
    head = re.search(r"fn " + fn + r"\(.*?\)\s*->\s*" + re.escape(old_ret) + r"\s*\{", src, re.S)
    if not head:
        sys.exit(f"{path}: no `fn {fn}` returning {old_ret}")
    depth, i = 1, head.end()
    while depth:
        depth += {"{": 1, "}": -1}.get(src[i], 0)
        i += 1
    body = src[head.end():i - 1]
    sig = src[head.start():head.end()].replace(old_ret, new_ret)
    new = f"{sig}\n        let r: {old_ret} = (|| {{{body}}})();\n        r.map(|()| {ok_map})\n    }}"
    open(path, "w").write(src[:head.start()] + new + src[i:])


root = sys.argv[1]
timing = "zencodec::encode::OutputTiming::AT_FINISH"
for f in ["pnm_codec.rs", "farbfeld_codec.rs", "qoi_codec.rs"]:
    wrap(f"{root}/zenbitmaps-main/src/codec/{f}", "begin", "Result<(), Self::Error>",
         "Result<zencodec::encode::OutputTiming, Self::Error>", timing)
wrap(f"{root}/imij-main/src/codec/encode.rs", "push_frame_timed", "Result<(), Error>",
     "Result<zencodec::animation::FrameDuration, Error>", "duration")
