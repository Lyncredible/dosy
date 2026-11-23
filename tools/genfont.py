"""Generate CP437 font tables from a BDF file. Usage: genfont.py in.bdf out.rs"""
import sys

CTRL = "\u0000☺☻♥♦♣♠•◘○◙♂♀♪♫☼►◄↕‼¶§▬↨↑↓→←∟↔▲▼"
HIGH = ("ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»"
        "░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀"
        "αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■ ")
CP437 = CTRL + "".join(chr(c) for c in range(0x20, 0x7F)) + "⌂" + HIGH
assert len(CP437) == 256, len(CP437)

W, H = 8, 16
ascent = None
glyphs = {}
cur = None
lines = open(sys.argv[1], encoding="latin-1").read().splitlines()
i = 0
while i < len(lines):
    l = lines[i]
    if l.startswith("FONTBOUNDINGBOX"):
        _, fw, fh, fx, fy = l.split()
        ascent = int(fh) + int(fy)  # baseline row from top
    elif l.startswith("ENCODING"):
        cur = {"enc": int(l.split()[1])}
    elif l.startswith("BBX"):
        _, bw, bh, bx, by = l.split()
        cur["bbx"] = (int(bw), int(bh), int(bx), int(by))
    elif l == "BITMAP":
        bw, bh, bx, by = cur["bbx"]
        rows = [int(lines[i + 1 + k], 16) for k in range(bh)]
        nbytes = (bw + 7) // 8
        g = [0] * H
        top = ascent - (by + bh)
        for k, r in enumerate(rows):
            y = top + k
            if 0 <= y < H:
                bits = r >> (nbytes * 8 - 8) if nbytes > 1 else r
                g[y] = (bits >> bx) & 0xFF if bx >= 0 else (bits << -bx) & 0xFF
        glyphs[cur["enc"]] = g
        i += bh
    i += 1

out16, missing = [], []
for code, ch in enumerate(CP437):
    g = glyphs.get(ord(ch))
    alt = {0x10: 0x25B6, 0x11: 0x25C0, 0x1C: 0x2319}
    if g is None and code in alt:
        g = glyphs.get(alt[code])
    hand = {
        0x10: [0, 0, 0x80, 0xC0, 0xE0, 0xF0, 0xF8, 0xFE, 0xF8, 0xF0, 0xE0, 0xC0, 0x80, 0, 0, 0],
        0x11: [0, 0, 0x02, 0x06, 0x0E, 0x1E, 0x3E, 0xFE, 0x3E, 0x1E, 0x0E, 0x06, 0x02, 0, 0, 0],
    }
    if g is None and code in hand:
        g = hand[code]
    if g is None:
        if code not in (0, 0x20, 0xFF):
            missing.append(f"{code:02X}")
        g = [0] * H
    out16.append(g)
# 8x8: OR each pair of rows of the 8x16 glyph.
out8 = [[g[2 * k] | g[2 * k + 1] for k in range(8)] for g in out16]

def table(name, data, h):
    s = [f"pub static {name}: [u8; {256 * h}] = ["]
    for code, g in enumerate(data):
        s.append("    " + ", ".join(f"0x{b:02X}" for b in g) + f", // {code:02X}")
    s.append("];")
    return "\n".join(s)

with open(sys.argv[2], "w", encoding="utf-8", newline="\n") as f:
    f.write("""//! Built-in CP437 fonts (generated; do not edit by hand).
//!
//! Glyphs are from Spleen 8x16 by Frederic Cambus
//! (<https://github.com/fcambus/spleen>), BSD-2-Clause:
//!
""" + "".join(("//! " + l).rstrip() + "\n" for l in open(sys.argv[3], encoding="utf-8").read().splitlines()) + """//!
//! CP437 codes are mapped to Unicode and looked up in the BDF. The 8x8 font
//! ORs row pairs of the 8x16 glyphs. 10h/11h (triangles) are hand-drawn.
""")
    if missing:
        f.write(f"//! Missing in the source (blank): {' '.join(missing)}\n")
    f.write("\n/// 8x16 glyphs, 16 bytes per character, MSB = leftmost pixel.\n")
    f.write("#[rustfmt::skip]\n" + table("FONT_8X16", out16, 16) + "\n\n")
    f.write("/// 8x8 glyphs, 8 bytes per character.\n")
    f.write("#[rustfmt::skip]\n" + table("FONT_8X8", out8, 8) + "\n")
print("missing:", missing)
