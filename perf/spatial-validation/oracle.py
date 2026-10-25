"""Required independent TIFF readback through Pillow's libtiff decoder."""
import sys
from pathlib import Path

import PIL
from PIL import Image, TiffImagePlugin, features

assert PIL.__version__ == "13.0.0", PIL.__version__
assert features.check_codec("libtiff"), "Pillow must include libtiff"
TiffImagePlugin.READ_LIBTIFF = True
with Image.open(sys.argv[1]) as image:
    assert image.mode == "RGB", image.mode
    assert image.tile[0].codec_name == "libtiff", image.tile
    image.load()
    with Path(sys.argv[2]).open("xb") as out:
        out.write(image.tobytes())
print(f"Pillow={PIL.__version__} libtiff={features.version_codec('libtiff')}")
