"""Builds the derived icon files from the masters in this folder.

    python3 -I assets/icon/generate.py

Needs `rsvg-convert` (librsvg) on the PATH. Writes, next to this script:

- senzor-{40,48,64,128,256,512}.png  sizes rendered from senzor.svg (the hand-tuned
                                      16/20/22/24/32 PNGs are used as they are);
- senzor.ico                          Windows: 16, 20, 24, 32, 40, 48, 64 and 256 px;
- senzor.icns                         macOS: 16 to 1024 px, from senzor-macos.svg.

and the docs site's icons in website/app/ (Next.js picks them up by name):

- favicon.ico                         16, 32 and 48 px;
- icon.svg                            a copy of senzor.svg;
- apple-icon.png                      180 px on the tile colour, without transparent corners.

Run it again after changing a master and commit the results.
"""

import pathlib
import re
import shutil
import struct
import subprocess
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
MASTER = HERE / "senzor.svg"
MACOS_MASTER = HERE / "senzor-macos.svg"
WEBSITE_APP = HERE.parent.parent / "website" / "app"

# Sizes drawn by hand (src/senzor-<n>.svg -> senzor-<n>.png); never rendered from the master.
HAND_TUNED = (16, 20, 22, 24, 32)
RENDERED = (40, 48, 64, 128, 256, 512)
ICO_SIZES = (16, 20, 24, 32, 40, 48, 64, 256)
# ICNS entry types with PNG payloads, and the pixel size of each.
ICNS_TYPES = (
    (b"icp4", 16),
    (b"icp5", 32),
    (b"ic11", 32),  # 16@2x
    (b"icp6", 64),
    (b"ic12", 64),  # 32@2x
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic13", 256),  # 128@2x
    (b"ic09", 512),
    (b"ic14", 512),  # 256@2x
    (b"ic10", 1024),  # 512@2x
)


def render(svg: pathlib.Path, size: int, out: pathlib.Path, background: str | None = None) -> None:
    command = ["rsvg-convert", "--width", str(size), "--height", str(size), "--output", str(out)]
    if background:
        command += ["--background-color", background]
    subprocess.run(command + [str(svg)], check=True)


def tile_colour() -> str:
    """The fill of the master's first shape, its rounded tile."""
    match = re.search(r'<rect[^>]*fill="(#[0-9a-fA-F]{6})"', MASTER.read_text())
    assert match, "senzor.svg starts with a filled <rect>"
    return match.group(1)


def png(size: int) -> bytes:
    return (HERE / f"senzor-{size}.png").read_bytes()


def png_size(data: bytes) -> tuple[int, int]:
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
    return struct.unpack(">II", data[16:24])


def write_ico(sizes, out: pathlib.Path) -> None:
    """An ICO whose images are all PNG-compressed (supported since Windows Vista)."""
    images = [png(size) for size in sizes]
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = len(header) + 16 * len(images)
    entries, payload = b"", b""
    for size, data in zip(sizes, images):
        assert png_size(data) == (size, size), f"senzor-{size}.png is not {size}x{size}"
        dim = 0 if size >= 256 else size  # 0 means 256
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset + len(payload))
        payload += data
    out.write_bytes(header + entries + payload)


def write_icns(out: pathlib.Path) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        rendered = {}
        for size in sorted({size for _, size in ICNS_TYPES}):
            path = pathlib.Path(tmp) / f"{size}.png"
            render(MACOS_MASTER, size, path)
            rendered[size] = path.read_bytes()
    body = b"".join(
        kind + struct.pack(">I", 8 + len(rendered[size])) + rendered[size] for kind, size in ICNS_TYPES
    )
    out.write_bytes(b"icns" + struct.pack(">I", 8 + len(body)) + body)


def main() -> None:
    for size in HAND_TUNED:
        assert png_size(png(size)) == (size, size), f"senzor-{size}.png is not {size}x{size}"
    for size in RENDERED:
        render(MASTER, size, HERE / f"senzor-{size}.png")
    write_ico(ICO_SIZES, HERE / "senzor.ico")
    write_icns(HERE / "senzor.icns")
    write_ico((16, 32, 48), WEBSITE_APP / "favicon.ico")
    shutil.copyfile(MASTER, WEBSITE_APP / "icon.svg")
    render(MASTER, 180, WEBSITE_APP / "apple-icon.png", background=tile_colour())
    print("wrote", ", ".join(f"senzor-{s}.png" for s in RENDERED), "senzor.ico and senzor.icns")
    print("wrote website/app/favicon.ico, icon.svg and apple-icon.png")


main()
