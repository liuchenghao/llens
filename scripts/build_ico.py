#!/usr/bin/env python3
"""生成标准 .ico（内嵌 16/32/48/64/256 五种尺寸 PNG）。

用法（需先准备 5 个尺寸的 PNG）：
  python3 build_ico.py <out.ico> <16.png> <32.png> <48.png> <64.png> <256.png>

ICONDIRENTRY 字段布局（小端）：
  w(1) h(1) colors(1)=0 reserved(1)=0 planes(2)=1 bitcount(2)=32 size(4) offset(4)

macOS 本机若没有 sips 缩放好的 5 张 PNG，可先：
  sips -z 16 16 icon.png --out /tmp/i16.png   # 及 32/48/64/256
再用 sips -g pixelWidth 校验后调用本脚本。
"""
import struct
import sys


def build_ico(out: str, pngs: list[str], sizes: list[int]) -> None:
    data = [open(p, "rb").read() for p in pngs]
    n = len(sizes)
    header = struct.pack("<HHH", 0, 1, n)
    entries = b""
    body = b""
    offset = 6 + n * 16
    for i, sz in enumerate(sizes):
        wh = 0 if sz >= 256 else sz
        entries += struct.pack("<BBBBHHII", wh, wh, 0, 0, 1, 32, len(data[i]), offset)
        body += data[i]
        offset += len(data[i])
    with open(out, "wb") as f:
        f.write(header + entries + body)
    print(f"OK {out}: {len(header) + len(entries) + len(body)} bytes")
    # 校验
    blob = open(out, "rb").read()
    for i in range(n):
        e = 6 + i * 16
        w, h, cc, rv, pl, bc = struct.unpack_from("<BBBBHH", blob, e)
        sz_f, off_f = struct.unpack_from("<II", blob, e + 8)
        assert rv == 0, f"entry{i}: reserved={rv}"
        assert pl == 1 and bc == 32, f"entry{i}: planes/bitcount 错"
        assert blob[off_f : off_f + 4] == b"\x89PNG", f"entry{i}: PNG 头缺失 @ {off_f}"
        print(f"  entry{i + 1} ({sizes[i]}px): reserved=0 planes=1 bits=32 size={sz_f} offset={off_f} PNG头=OK")


if __name__ == "__main__":
    if len(sys.argv) != 7:
        print(__doc__)
        sys.exit(1)
    build_ico(sys.argv[1], sys.argv[2:7], [16, 32, 48, 64, 256])
