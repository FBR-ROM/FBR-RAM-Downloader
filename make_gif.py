#!/usr/bin/env python3
"""Render the dumped truecolor TUI frames to an animated GIF (M+ Nerd Font Mono)."""
import re, sys
from PIL import Image, ImageDraw, ImageFont

FONT = "/nix/store/zrbky24b568vx4filjjnk0i6xflnviay-nerd-fonts-m+-3.5.0+1.100/share/fonts/truetype/NerdFonts/M+/M+CodeLat60NerdFontMono-Regular.ttf"
BG = (0, 0, 0)

def load_frames(path):
    with open(path) as f:
        text = f.read()
    return [b for b in text.split("\n\n") if b.strip()]

def parse_frame(frame):
    """Parse a truecolor ANSI frame into (char, fg, bold, bg) cells."""
    lines = frame.split("\n")
    grid = []
    for line in lines:
        cells = []
        fg = (0, 0, 0)
        bg = None
        bold = False
        i = 0
        while i < len(line):
            if line[i] == "\x1b" and i + 1 < len(line) and line[i+1] == "[":
                m = re.match(r"\x1b\[([0-9;]*)m", line[i:])
                if m:
                    parts = m.group(1).split(";")
                    for code in parts:
                        if code == "1":
                            bold = True
                        elif code in ("22", "49"):
                            if code == "22":
                                bold = False
                            if code == "49":
                                bg = None
                        elif code == "0":
                            fg = (0, 0, 0); bg = None; bold = False
                    # handle 38;2;r;g;b and 48;2;r;g;b
                    for base, setter in (("38", "fg"), ("48", "bg")):
                        if base in parts:
                            j = parts.index(base)
                            if parts[j+1] == "2" and len(parts) >= j+4:
                                col = (int(parts[j+2]), int(parts[j+3]), int(parts[j+4]))
                                if base == "38":
                                    fg = col
                                else:
                                    bg = col
                    i += m.end()
                    continue
            cells.append((line[i], fg, bold, bg))
            i += 1
        grid.append(cells)
    return grid

def measure(font):
    tmp = Image.new("RGB", (10, 10))
    d = ImageDraw.Draw(tmp)
    adv = d.textlength("M", font=font)
    asc, desc = font.getmetrics()
    return int(round(adv)), asc + desc

def render_frame(grid, font, cw, ch):
    w = max(len(r) for r in grid)
    h = len(grid)
    img = Image.new("RGB", (w * cw, h * ch), BG)
    d = ImageDraw.Draw(img)
    half = ch // 2
    for r, row in enumerate(grid):
        for c, (ch_, fg, bold, bg) in enumerate(row):
            x = c * cw
            y = r * ch
            # Half-block support: `▀` paints fg on the top half and bg on the
            # bottom half of the cell.
            if ch_ == "▀" and bg is not None:
                d.rectangle([x, y + half, x + cw - 1, y + ch - 1], fill=bg)
            if ch_ == " ":
                continue
            # Bold: brighten.
            col = tuple(min(255, int(v * 1.35)) for v in fg) if bold else fg
            d.text((x, y), ch_, font=font, fill=col)
    return img

def main():
    frames_text = load_frames(sys.argv[1])
    step = int(sys.argv[2]) if len(sys.argv) > 2 else 2
    frames_text = frames_text[::step]
    font = ImageFont.truetype(FONT, 16)
    cw, ch = measure(font)
    imgs = []
    for f in frames_text:
        grid = parse_frame(f)
        imgs.append(render_frame(grid, font, cw, ch))
    W = max(i.width for i in imgs)
    H = max(i.height for i in imgs)
    norm = []
    for i in imgs:
        canvas = Image.new("RGB", (W, H), BG)
        canvas.paste(i, (0, 0))
        norm.append(canvas)
    out = sys.argv[3] if len(sys.argv) > 3 else "preview.gif"
    norm[0].save(out, save_all=True, append_images=norm[1:], duration=60, loop=0, optimize=True)
    print(f"wrote {out}: {len(norm)} frames, {W}x{H}, cell {cw}x{ch}")

if __name__ == "__main__":
    main()
