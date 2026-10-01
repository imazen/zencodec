#!/usr/bin/env python3
"""Check zenmeta's edited XMP using independent XML and ExifTool readers."""
import json
import subprocess
import sys
import xml.etree.ElementTree as ET

path = sys.argv[1]
ns = "http://ns.google.com/photos/1.0/container/item/"
container = "http://ns.google.com/photos/1.0/container/"
items = ET.parse(path).findall(f".//{{{container}}}Item")
assert [i.attrib[f"{{{ns}}}Semantic"] for i in items] == ["Primary", "GainMap"]
assert items[1].attrib[f"{{{ns}}}Length"] == "123"
result = subprocess.run(
    ["exiftool", "-j", "-struct", "-XMP-GContainer:all", path],
    check=True, capture_output=True, text=True,
)
report = json.loads(result.stdout)
assert report[0]["ContainerDirectory"][1]["Item"]["Length"] == 123, report
assert report[0]["ContainerDirectory"][1]["Item"]["Semantic"] == "GainMap", report
print("ElementTree and ExifTool agree: GainMap length 123; Primary then GainMap")
