#!/usr/bin/env python3
"""Check shared pixel identity at codec interfaces in resolved Cargo metadata.

Use metadata from the actual test feature/target selection, outside workspace
path overrides. This checks source IDs, not just version numbers. It does not
require unrelated packages to abandon older zenpixels versions.
"""
import json
import sys

FAMILY = {"zencodec", "zencodec-io", "zencodec-coroutine", "zencodec-media",
          "zencodec-testkit", "zencodec-media-testkit", "zenmeta", "zencodec-render"}


def check(metadata):
    if not metadata.get("resolve"):
        raise ValueError("resolved metadata required (do not use --no-deps)")
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}

    def named_dependencies(ident, names):
        return {dep for dep in nodes[ident]["dependencies"]
                if packages[dep]["name"] in names}

    def cores(ident, seen):
        if ident in seen:
            return set()
        if packages[ident]["name"] == "zencodec":
            return {ident}
        seen = seen | {ident}
        found = set()
        for dep in named_dependencies(ident, FAMILY):
            found |= cores(dep, seen)
        return found

    errors = []
    checked = 0
    for ident in nodes:
        core_ids = cores(ident, set())
        if not core_ids:
            continue
        checked += 1
        if len(core_ids) != 1:
            errors.append(f"{ident}: multiple core identities: {sorted(core_ids)}")
            continue
        core = next(iter(core_ids))
        expected = named_dependencies(core, {"zenpixels"})
        if len(expected) != 1:
            errors.append(f"{core}: expected one pixel identity, found {sorted(expected)}")
            continue
        actual = named_dependencies(ident, {"zenpixels"})
        if actual and actual != expected:
            errors.append(f"{ident}: pixels {sorted(actual)} disagree with core pixels {sorted(expected)}")
        for converter in named_dependencies(ident, {"zenpixels-convert"}):
            converted = named_dependencies(converter, {"zenpixels"})
            if converted != expected:
                errors.append(f"{ident}: converter {converter} uses {sorted(converted)}, core uses {sorted(expected)}")
    if not checked:
        raise ValueError("no resolved zencodec interfaces found")
    return checked, errors


def main():
    try:
        with open(sys.argv[1], encoding="utf-8") as source:
            checked, errors = check(json.load(source))
    except (IndexError, KeyError, OSError, ValueError) as error:
        print(f"codec graph: invalid input: {error}", file=sys.stderr)
        return 2
    for error in errors:
        print(error, file=sys.stderr)
    print(f"codec graph: {checked} interface consumers, {len(errors)} mismatches")
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
