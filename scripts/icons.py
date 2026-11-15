"""Helpers for Glyph's own icons (assets/icons/ui). See docs/ICONS.md.

  python scripts/icons.py check              check every icon against the rules
  python scripts/icons.py preview [out.html] page with all icons (default target/icons.html)
  python scripts/icons.py new <name>         new icon file from the template
  python scripts/icons.py arrow 4,12 20,12   path data for a shape (see SHAPES below)

The shape commands print SVG elements you paste into an icon file.
"""
import math
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DIR = os.path.join(ROOT, "assets", "icons", "ui")
HEAD = (
    '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" '
    'fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">'
)
NOTE = "<!-- Glyph icon set, see docs/ICONS.md -->"


# ---- numbers -------------------------------------------------------------

def n(v):
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def pt(p):
    return f"{n(p[0])} {n(p[1])}"


def polar(r, deg, c=(12, 12)):
    a = math.radians(deg)
    return (c[0] + r * math.cos(a), c[1] + r * math.sin(a))


def unit(x, y):
    m = math.hypot(x, y)
    return (x / m, y / m)


def _cut(v, q, r):
    dx, dy = q[0] - v[0], q[1] - v[1]
    d = math.hypot(dx, dy)
    m = min(r, d / 2)
    return (v[0] + dx / d * m, v[1] + dy / d * m)


# ---- shapes --------------------------------------------------------------

def round_shape(points, r=2):
    """Closed shape through `points`, every corner rounded by `r`."""
    k = len(points)
    cuts = [(_cut(points[i], points[i - 1], r), points[i], _cut(points[i], points[(i + 1) % k], r)) for i in range(k)]
    d = f"M{pt(cuts[0][2])}"
    for i in list(range(1, k)) + [0]:
        a, v, b = cuts[i]
        d += f"L{pt(a)}Q{pt(v)} {pt(b)}"
    return d + "Z"


def round_line(points, r=2.2):
    """Open line through `points`, inner corners rounded by `r`."""
    d = f"M{pt(points[0])}"
    for i in range(1, len(points) - 1):
        v = points[i]
        d += f"L{pt(_cut(v, points[i - 1], r))}Q{pt(v)} {pt(_cut(v, points[i + 1], r))}"
    return d + f"L{pt(points[-1])}"


def path(d, fill=False):
    return f'<path{" fill=\"currentColor\"" if fill else ""} d="{d}"/>'


def head(base, d, length=4.6, width=8.4):
    """Filled, rounded arrow head: base centre `base`, pointing along `d`."""
    px, py = -d[1], d[0]
    tip = (base[0] + d[0] * length, base[1] + d[1] * length)
    a = (base[0] + px * width / 2, base[1] + py * width / 2)
    b = (base[0] - px * width / 2, base[1] - py * width / 2)
    return path(round_shape([tip, a, b], 1.3), fill=True)


def arrow(tail, tip, length=4.6, width=8.4):
    """Straight arrow: a line plus a filled head whose point is `tip`."""
    d = unit(tip[0] - tail[0], tip[1] - tail[1])
    base = (tip[0] - d[0] * length, tip[1] - d[1] * length)
    return [path(f"M{pt(tail)}L{pt(base)}"), head(base, d, length, width)]


def arc_arrow(start, end, cw=True, r=8.2, c=(12, 12), length=4.2, width=7.6):
    """Arc around `c` from angle `start` to `end` (degrees; 0 = right,
    90 = down), filled head at `end`. `cw`: clockwise on screen."""
    span = (end - start) % 360 if cw else (start - end) % 360
    back = math.degrees(length * 0.35 / r)
    e2 = end - back if cw else end + back
    p0, p1 = polar(r, start, c), polar(r, e2, c)
    a = math.radians(e2)
    d = (-math.sin(a), math.cos(a)) if cw else (math.sin(a), -math.cos(a))
    arc = f"M{pt(p0)}A{n(r)} {n(r)} 0 {1 if span - back > 180 else 0} {1 if cw else 0} {pt(p1)}"
    return [path(arc), head(p1, d, length, width)]


def star(tips=4, r_out=8, r_in=2.6, c=(12, 12), r=1.6):
    """Soft star (sparkle), filled."""
    pts = [polar(r_out if i % 2 == 0 else r_in, -90 + i * 180 / tips, c) for i in range(tips * 2)]
    return path(round_shape(pts, r), fill=True)


def gear(teeth=6, r_out=9.5, r_in=7.2, top=10, base=17, r=1.4):
    pts = []
    for i in range(teeth):
        a = -90 + i * 360 / teeth
        pts += [polar(r_in, a - base), polar(r_out, a - top), polar(r_out, a + top), polar(r_in, a + base)]
    return path(round_shape(pts, r))


SHAPES = {
    "arrow": "arrow TAIL TIP              e.g. arrow 4,12 20,12",
    "arc-arrow": "arc-arrow START END [ccw]   angles in degrees, e.g. arc-arrow 30 320",
    "round-shape": "round-shape R P1 P2 P3 ...  closed shape, corners rounded by R",
    "round-line": "round-line R P1 P2 P3 ...   open line, corners rounded by R",
    "star": "star [TIPS]",
    "gear": "gear [TEETH]",
}


# ---- files ---------------------------------------------------------------

def icons():
    return sorted(f[:-4] for f in os.listdir(DIR) if f.endswith(".svg"))


def check():
    bad = 0
    for name in icons():
        with open(os.path.join(DIR, name + ".svg"), encoding="utf-8") as f:
            src = f.read()
        problems = []
        if HEAD not in src:
            problems.append("header differs from the template")
        if re.search(r'stroke-width="(?!2")', src):
            problems.append("other stroke width than 2")
        if re.search(r'(fill|stroke)="#', src):
            problems.append("fixed colour (use currentColor)")
        if re.search(r"<(polygon|polyline)\b", src):
            problems.append("polygon/polyline: corners are not rounded, use a path")
        if name.endswith("-fill") and not os.path.exists(os.path.join(DIR, name[:-5] + ".svg")):
            problems.append("-fill icon without its outline version")
        for p in problems:
            print(f"{name}: {p}")
        bad += bool(problems)
    print(f"{len(icons())} icons, {bad} with problems")
    return bad == 0


def preview(out):
    cells = []
    for name in icons():
        with open(os.path.join(DIR, name + ".svg"), encoding="utf-8") as f:
            svg = f.read()
        cells.append(f'<div class=c><div class=r><i class=b>{svg}</i><i>{svg}</i><i class=s>{svg}</i></div><span>{name}</span></div>')
    page = (
        "<!doctype html><meta charset=utf-8><title>Glyph icons</title><style>"
        "body{margin:0;padding:16px;background:#030303;color:#f2f2f2;font:12px Inter,system-ui,sans-serif}"
        ".g{display:grid;grid-template-columns:repeat(auto-fill,minmax(150px,1fr));gap:8px}"
        ".c{background:rgba(20,20,20,.75);border-radius:10px;padding:10px}.r{display:flex;align-items:center;gap:14px}"
        "i{opacity:.92}i svg{width:20px;height:20px;display:block}i.b svg{width:48px;height:48px}i.s svg{width:14px;height:14px}"
        "span{display:block;margin-top:6px;color:rgba(242,242,242,.56)}"
        "</style><div class=g>" + "".join(cells) + "</div>"
    )
    os.makedirs(os.path.dirname(out) or ".", exist_ok=True)
    with open(out, "w", encoding="utf-8") as f:
        f.write(page)
    print(f"wrote {out}")


def new(name):
    dest = os.path.join(DIR, name + ".svg")
    if os.path.exists(dest):
        sys.exit(f"{name}.svg exists already")
    with open(dest, "w", encoding="utf-8", newline="\n") as f:
        f.write(f"{NOTE}\n{HEAD}\n  <circle cx=\"12\" cy=\"12\" r=\"9.5\"/>\n</svg>\n")
    print(f"created {dest}")


def _p(s):
    x, y = s.split(",")
    return (float(x), float(y))


def main(argv):
    if not argv:
        print(__doc__)
        print("shapes:\n  " + "\n  ".join(SHAPES.values()))
        return
    cmd, args = argv[0], argv[1:]
    if cmd == "check":
        sys.exit(0 if check() else 1)
    elif cmd == "preview":
        preview(args[0] if args else os.path.join(ROOT, "target", "icons.html"))
    elif cmd == "new":
        new(args[0])
    elif cmd == "arrow":
        print("\n".join(arrow(_p(args[0]), _p(args[1]))))
    elif cmd == "arc-arrow":
        print("\n".join(arc_arrow(float(args[0]), float(args[1]), cw="ccw" not in args)))
    elif cmd == "round-shape":
        print(path(round_shape([_p(a) for a in args[1:]], float(args[0]))))
    elif cmd == "round-line":
        print(path(round_line([_p(a) for a in args[1:]], float(args[0]))))
    elif cmd == "star":
        print(star(int(args[0]) if args else 4))
    elif cmd == "gear":
        print(gear(int(args[0]) if args else 6))
    else:
        sys.exit(f"unknown command {cmd}")


if __name__ == "__main__":
    main(sys.argv[1:])
