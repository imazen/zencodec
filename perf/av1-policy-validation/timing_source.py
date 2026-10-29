"""Fetch identical pinned FFmpeg source through bounded HTTPS transports."""
import hashlib
from pathlib import Path
import subprocess
import tempfile

SHA256 = {
    "6.1.1": "8684f4b00f94b85461884c3719382f1261f0d9eb3d59640a1f4ac0873616f968",
    "8.0.1": "05ee0b03119b45c0bdb4df654b96802e909e0a752f72e4fe3794f487229e5a41",
}
SOURCES = ("https://ffmpeg.org/releases", "https://mirrors.omnios.org/ffmpeg")


def verify(path, expected):
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    if digest != expected:
        raise ValueError(f"source checksum mismatch: {digest}, expected {expected}")
    return digest


def fetch_source(root, version):
    expected = SHA256[version]
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    archive = root / f"ffmpeg-{version}.tar.xz"
    if not archive.exists():
        for source in SOURCES:
            # Preserve every failed attempt. Never cache bytes before verification.
            with tempfile.NamedTemporaryFile(prefix=archive.name + ".partial-",
                                             dir=root, delete=False) as partial:
                download = Path(partial.name)
            url = f"{source}/{archive.name}"
            print(f"downloading pinned source from {url} to {download}", flush=True)
            try:
                subprocess.run([
                    "curl", "--fail", "--location", "--proto", "=https",
                    "--proto-redir", "=https", "--connect-timeout", "15",
                    "--max-time", "60", url, "-o", str(download),
                ], check=True, timeout=65)
            except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
                print(f"transport failed; retained {download}: {error}", flush=True)
                if source == SOURCES[-1]:
                    raise
                continue
            # Integrity failures are terminal, not a reason to try another host.
            verify(download, expected)
            download.rename(archive)
            break
    digest = verify(archive, expected)
    (root / "source.sha256").write_text(f"{digest}  {archive.name}\n")
    return archive
