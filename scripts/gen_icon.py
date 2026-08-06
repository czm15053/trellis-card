#!/usr/bin/env python3
"""生成 trellis-card 应用图标源图（1024x1024 PNG，无第三方依赖）。
暗色圆角方底 + 薄荷绿菱形 jewel（道岔视觉），供 `npx tauri icon` 使用。
"""
import struct
import zlib

S = 1024
R = 220  # 圆角半径

INK_TOP = (23, 26, 36)    # #171a24
INK_BOT = (12, 14, 20)    # #0c0e14
MINT = (69, 196, 160)     # #45c4a0
MINT_SOFT = (110, 217, 181)


def rounded_rect_mask(x, y):
    m = R
    cx = min(max(x, m), S - m)
    cy = min(max(y, m), S - m)
    dx, dy = x - cx, y - cy
    return dx * dx + dy * dy <= m * m


def diamond_dist(x, y, cx, cy, r):
    return abs(x - cx) + abs(y - cy) - r


def clamp(v):
    return max(0, min(255, int(v)))


def blend(dst, src, a):
    return tuple(clamp(d * (1 - a) + s * a) for d, s in zip(dst, src))


rows = []
cx, cy = S / 2, S / 2
for y in range(S):
    row = bytearray()
    for x in range(S):
        if not rounded_rect_mask(x, y):
            row += bytes((0, 0, 0, 0))
            continue
        # 纵向渐变底
        t = y / S
        base = blend(INK_TOP, INK_BOT, t)
        # 顶部微光
        glow = max(0.0, 1.0 - (((x - cx) ** 2 + (y - S * 0.18) ** 2) ** 0.5 / (S * 0.55)))
        base = blend(base, MINT_SOFT, glow * 0.10)
        a = 255
        # jewel：外层光晕 + 菱形
        d_outer = diamond_dist(x, y, cx, cy, 300)
        d_main = diamond_dist(x, y, cx, cy, 210)
        if d_outer < 0:
            halo = max(0.0, 1.0 + d_outer / 90)  # 越靠外越淡
            base = blend(base, MINT, halo * 0.35)
        if d_main < 0:
            edge = min(1.0, -d_main / 24)  # 边缘柔化
            base = blend(base, MINT, 0.95 * edge)
            # 中心高光
            hl = max(0.0, 1.0 + diamond_dist(x, y, cx - 40, cy - 40, 120) / 120)
            base = blend(base, (236, 255, 248), hl * 0.5 * edge)
        row += bytes((*base, a))
    rows.append(b"\x00" + bytes(row))

raw = b"".join(rows)


def chunk(tag, data):
    c = tag + data
    return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c))


png = (
    b"\x89PNG\r\n\x1a\n"
    + chunk(b"IHDR", struct.pack(">IIBBBBB", S, S, 8, 6, 0, 0, 0))
    + chunk(b"IDAT", zlib.compress(raw, 9))
    + chunk(b"IEND", b"")
)

out = "src-tauri/icons/icon-source.png"
import os
os.makedirs(os.path.dirname(out), exist_ok=True)
with open(out, "wb") as f:
    f.write(png)
print("written", out, len(png), "bytes")
