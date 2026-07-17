#!/usr/bin/env python3
"""Generates the Failure Build braille-art logo (logo07.txt / logo05.txt).

Renders a bold "F" monogram as a boolean pixel grid, then packs it into
Unicode Braille Pattern characters (U+2800-U+28FF), where each character
encodes a 2-wide x 4-tall block of dots:

    1 4
    2 5
    3 6
    7 8

Run `python3 generate_logo.py` from this directory (or anywhere — paths are
relative to the script) to regenerate both files after adjusting the shape
functions below. logo07.txt is the full (7-row) logo used in the hero box
and tall terminals; logo05.txt is the small (5-row) variant used at medium
terminal heights (see crates/codegen/xai-grok-pager/src/views/welcome/logo.rs).
"""

from pathlib import Path

# Bit offset for each (dx, dy) position within a braille cell's 2x4 dot grid.
DOT_BITS = {
    (0, 0): 0x01, (0, 1): 0x02, (0, 2): 0x04, (0, 3): 0x40,
    (1, 0): 0x08, (1, 1): 0x10, (1, 2): 0x20, (1, 3): 0x80,
}


def new_grid(width: int, height: int) -> list[list[bool]]:
    return [[False] * width for _ in range(height)]


def fill_rect(grid, x0: int, y0: int, x1: int, y1: int) -> None:
    """Fills the inclusive rectangle [x0, x1] x [y0, y1], clipped to the grid."""
    height, width = len(grid), len(grid[0])
    for y in range(max(0, y0), min(height, y1 + 1)):
        for x in range(max(0, x0), min(width, x1 + 1)):
            grid[y][x] = True


def to_braille(grid) -> str:
    """Packs a boolean pixel grid into rows of Braille characters."""
    height, width = len(grid), len(grid[0])
    lines = []
    for cell_y in range(0, height, 4):
        row_chars = []
        for cell_x in range(0, width, 2):
            code = 0
            for (dx, dy), bit in DOT_BITS.items():
                x, y = cell_x + dx, cell_y + dy
                if y < height and x < width and grid[y][x]:
                    code |= bit
            row_chars.append(chr(0x2800 + code))
        lines.append("".join(row_chars))
    return "\n".join(lines) + "\n"


def make_f_monogram(width: int, height: int) -> list[list[bool]]:
    """A bold geometric "F", proportioned for a roughly square dot canvas."""
    grid = new_grid(width, height)
    stem_w = max(2, round(width * 0.18))
    stroke_h = max(2, round(height * 0.16))
    margin_x = max(1, round(width * 0.10))
    margin_y = max(1, round(height * 0.06))

    # Vertical stem, full height.
    fill_rect(grid, margin_x, margin_y, margin_x + stem_w - 1, height - 1 - margin_y)
    # Top arm, full width.
    fill_rect(grid, margin_x, margin_y, width - 1 - margin_x, margin_y + stroke_h - 1)
    # Middle arm, shorter — stops before the stem's midline overhang.
    mid_y = margin_y + round(height * 0.42)
    fill_rect(
        grid,
        margin_x,
        mid_y,
        width - 1 - margin_x - round(width * 0.18),
        mid_y + stroke_h - 1,
    )
    return grid


def main() -> None:
    here = Path(__file__).parent
    full = make_f_monogram(28, 28)  # 14 cols x 7 rows of braille cells
    small = make_f_monogram(20, 20)  # 10 cols x 5 rows of braille cells
    (here / "logo07.txt").write_text(to_braille(full), encoding="utf-8")
    (here / "logo05.txt").write_text(to_braille(small), encoding="utf-8")
    print("wrote logo07.txt and logo05.txt")


if __name__ == "__main__":
    main()
