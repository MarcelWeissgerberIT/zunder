#!/usr/bin/env python3
"""Bereitet die über Higgsfield generierten Icons für die App auf.

Eingang:  assets/higgsfield-raw/<name>/<name>.svg   (Toolbar-Icons, unverändert wie generiert)
          assets/higgsfield-raw/appicon-c/appicon-c.png (App-Icon-Motiv, vollflächig)
Ausgang:  ui/icons.svg          Sprite mit einem <symbol> je Icon (Farbe = currentColor)
          ui/appicon.png        App-Icon für die Startseite
          assets/Zunder.icns    App-Icon für das Bundle
          <work>/sheet.png      Kontrollbogen aller Icons (nur zur Sichtprüfung)

Die SVGs stammen aus einem Bildgenerator und gelten als nicht vertrauenswürdig:
Es wird nur eine feste Liste von Formen und Attributen übernommen, alles andere
(Skripte, Verweise, Stile, Einbettungen) fällt weg.

Aufruf:  python3 -I scripts/build_icons.py <arbeitsverzeichnis>
"""
import math
import os
import re
import site
import subprocess
import sys
import xml.etree.ElementTree as ET

# Das Skript läuft isoliert (python3 -I). Pillow liegt meist in den Nutzer-Paketen,
# die im isolierten Modus fehlen – diesen einen Pfad gezielt wieder zulassen.
_user_site = site.getusersitepackages()
if os.path.isdir(_user_site) and _user_site not in sys.path:
    sys.path.append(_user_site)

from PIL import Image, ImageChops, ImageDraw, ImageFilter

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RAW = os.path.join(ROOT, "assets", "higgsfield-raw")
UI = os.path.join(ROOT, "ui")
VB = 1024.0
STROKE = 72.0  # Zielstrichstärke in Einheiten der 1024er-Fläche

# Zielgröße (größte Ausdehnung inkl. Strich) als Anteil der Fläche.
DEFAULT_SIZE = 0.72
SIZES = {
    "close": 0.46,
    "plus": 0.56,
    "minus": 0.56,
    "check": 0.60,
    "chevron-down": 0.50,
    "menu": 0.62,
    "back": 0.62,
    "forward": 0.62,
    "search": 0.68,
    "reload": 0.70,
    "download": 0.68,
    "private": 0.80,
    "star": 0.76,
    "star-filled": 0.76,
    "code": 0.76,
    "offline": 0.78,
    "sparkle": 0.78,
    "send": 0.60,
    "stop": 0.50,
    "compare": 0.66,
    "image": 0.72,
    "key": 0.74,
    "eye": 0.78,
    "eye-off": 0.78,
    "user": 0.70,
    "upload": 0.68,
    "tabs": 0.64,
    "list": 0.68,
    "edit": 0.70,
    "grid": 0.66,
    "phone": 0.74,
    "folder-plus": 0.74,
    "stealth": 0.74,
}

SHAPES = {"path", "circle", "ellipse", "line", "rect", "polyline", "polygon"}
GEOM_ATTRS = {
    "d", "cx", "cy", "r", "rx", "ry", "x", "y", "x1", "y1", "x2", "y2",
    "width", "height", "points",
}
NUM = re.compile(r"^[0-9eE+\-., \t\r\n]*$")
PATH = re.compile(r"^[0-9eE+\-., \t\r\nMmLlHhVvCcSsQqTtAaZz]*$")


def local(tag):
    return tag.rsplit("}", 1)[-1]


def is_dark(value):
    v = (value or "").strip().lower()
    return v in ("#000", "#000000", "black", "currentcolor", "rgb(0,0,0)", "rgb(0, 0, 0)")


def sanitize(svg_path):
    """Liest ein generiertes SVG und gibt (viewbox, [(tag, attrs, filled)]) zurück."""
    data = open(svg_path, "rb").read()
    if len(data) > 64 * 1024:
        raise ValueError("zu groß")
    text = data.decode("utf-8")
    if "<!DOCTYPE" in text or "<!ENTITY" in text:
        raise ValueError("DOCTYPE/ENTITY nicht erlaubt")
    root = ET.fromstring(text)
    if local(root.tag) != "svg":
        raise ValueError("kein SVG")
    vb = [float(x) for x in re.split(r"[ ,]+", root.get("viewBox", "0 0 1024 1024").strip())]
    if len(vb) != 4 or vb[2] <= 0 or vb[3] <= 0:
        raise ValueError("viewBox ungültig")

    shapes = []

    def walk(node, inherited_fill, inherited_stroke):
        for child in node:
            tag = local(child.tag)
            fill = child.get("fill", inherited_fill)
            stroke = child.get("stroke", inherited_stroke)
            if tag == "g":
                if child.get("transform"):
                    raise ValueError("transform in Gruppe nicht unterstützt")
                walk(child, fill, stroke)
                continue
            if tag not in SHAPES:
                continue  # alles andere (script, style, image, use, text …) verwerfen
            if child.get("transform"):
                raise ValueError("transform nicht unterstützt")
            attrs = {}
            for key, value in child.attrib.items():
                key = local(key)
                if key not in GEOM_ATTRS:
                    continue
                pattern = PATH if key == "d" else NUM
                if not pattern.match(value) or len(value) > 8000:
                    raise ValueError(f"Attribut {key} enthält unerwartete Zeichen")
                attrs[key] = " ".join(value.split())
            # SVG-Vorgaben: ohne Angabe schwarz gefüllt, ohne Strich.
            fill_v = fill.strip().lower() if fill is not None else "black"
            stroke_v = stroke.strip().lower() if stroke is not None else "none"
            filled, stroked = is_dark(fill_v), is_dark(stroke_v)
            # Weiße oder farbige Flächen (z. B. ein Hintergrund) fallen weg.
            if fill_v not in ("none", "transparent") and not filled:
                continue
            if stroke_v not in ("none", "transparent") and not stroked:
                continue
            if not filled and not stroked:
                continue
            shapes.append((tag, attrs, filled, stroked))

    walk(root, None, None)
    if not shapes:
        raise ValueError("keine Formen gefunden")
    return vb, shapes


def shapes_markup(shapes, stroke_width, color):
    out = []
    for tag, attrs, filled, stroked in shapes:
        parts = [f'{k}="{v}"' for k, v in sorted(attrs.items())]
        parts.append(f'fill="{color if filled else "none"}"')
        if stroked:
            parts.append(f'stroke="{color}"')
            parts.append(f'stroke-width="{stroke_width:.2f}"')
        out.append(f"<{tag} {' '.join(parts)}/>")
    return "".join(out)


def render(svg_text, work, name, size=512):
    """Rendert ein SVG über Quick Look und gibt ein Graustufenbild zurück."""
    src_dir = os.path.join(work, "render-src")
    out_dir = os.path.join(work, "render-out")
    os.makedirs(src_dir, exist_ok=True)
    os.makedirs(out_dir, exist_ok=True)
    src = os.path.join(src_dir, f"{name}.svg")
    with open(src, "w", encoding="utf-8") as fh:
        fh.write(svg_text)
    subprocess.run(
        ["/usr/bin/qlmanage", "-t", "-s", str(size), "-o", out_dir, src],
        check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    return Image.open(os.path.join(out_dir, f"{name}.svg.png")).convert("RGBA")


def ink_bbox(img):
    """Begrenzungsrahmen der dunklen Pixel (auf weißem oder transparentem Grund)."""
    bg = Image.new("RGBA", img.size, (255, 255, 255, 255))
    flat = Image.alpha_composite(bg, img).convert("L")
    inv = ImageChops.invert(flat).point(lambda p: 255 if p > 40 else 0)
    return inv.getbbox()


def build_symbols(work):
    names = sorted(
        d for d in os.listdir(RAW)
        if os.path.isfile(os.path.join(RAW, d, f"{d}.svg"))
    )
    symbols = []
    report = []
    for name in names:
        vb, shapes = sanitize(os.path.join(RAW, name, f"{name}.svg"))
        unit = vb[2] / VB  # Maßstab der Quelle relativ zur 1024er-Fläche
        base_stroke = STROKE * unit
        probe = (
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vb[0]} {vb[1]} {vb[2]} {vb[3]}" '
            f'width="512" height="512"><rect x="{vb[0]}" y="{vb[1]}" width="{vb[2]}" height="{vb[3]}" fill="#fff"/>'
            f'<g stroke-linecap="round" stroke-linejoin="round">'
            f"{shapes_markup(shapes, base_stroke, '#000')}</g></svg>"
        )
        img = render(probe, work, name)
        box = ink_bbox(img)
        if not box:
            raise ValueError(f"{name}: leeres Bild")
        k = vb[2] / img.size[0]
        x0, y0, x1, y1 = (box[0] * k + vb[0], box[1] * k + vb[1], box[2] * k + vb[0], box[3] * k + vb[1])
        w, h = x1 - x0, y1 - y0
        target = SIZES.get(name, DEFAULT_SIZE) * VB
        # Der Strich soll nach dem Skalieren wieder STROKE breit sein; die reine
        # Geometrie (ohne Strich) wird deshalb auf (target - STROKE) gebracht.
        geom = max(w, h) - base_stroke
        s = (target - STROKE) / geom if geom > 1 else 1.0
        cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
        tx, ty = VB / 2 - cx * s, VB / 2 - cy * s
        stroke = STROKE / s
        body = shapes_markup(shapes, stroke, "currentColor")
        symbols.append(
            f'<symbol id="i-{name}" viewBox="0 0 1024 1024">'
            f'<g transform="translate({tx:.2f} {ty:.2f}) scale({s:.4f})" '
            f'stroke-linecap="round" stroke-linejoin="round">{body}</g></symbol>'
        )
        report.append((name, round(w / unit), round(h / unit), round(s, 3)))
    return names, symbols, report


def write_sprite(symbols):
    os.makedirs(UI, exist_ok=True)
    sprite = (
        '<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0" '
        'style="position:absolute" aria-hidden="true">' + "".join(symbols) + "</svg>\n"
    )
    with open(os.path.join(UI, "icons.svg"), "w", encoding="utf-8") as fh:
        fh.write(sprite)


def write_sheet(names, symbols, work):
    """Kontrollbogen: alle Icons groß und in Originalgröße nebeneinander."""
    cols = 6
    cell = 150
    rows = math.ceil(len(names) / cols)
    width, height = cols * cell, rows * cell
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}">',
        f'<rect width="{width}" height="{height}" fill="#ffffff"/>',
        "<defs>" + "".join(symbols) + "</defs>",
    ]
    for i, name in enumerate(names):
        x, y = (i % cols) * cell, (i // cols) * cell
        parts.append(f'<rect x="{x + 0.5}" y="{y + 0.5}" width="{cell - 1}" height="{cell - 1}" fill="none" stroke="#e5e5e5"/>')
        parts.append(f'<use href="#i-{name}" x="{x + 20}" y="{y + 12}" width="88" height="88" color="#1e1d1b"/>')
        parts.append(f'<use href="#i-{name}" x="{x + 116}" y="{y + 20}" width="22" height="22" color="#1e1d1b"/>')
        parts.append(f'<use href="#i-{name}" x="{x + 118}" y="{y + 56}" width="16" height="16" color="#1e1d1b"/>')
        parts.append(
            f'<text x="{x + cell / 2}" y="{y + 132}" font-family="Helvetica" font-size="13" '
            f'text-anchor="middle" fill="#555">{name}</text>'
        )
    parts.append("</svg>")
    img = render("".join(parts), work, "sheet", size=max(width, height))
    img.save(os.path.join(work, "sheet.png"))


def squircle_mask(size, scale=4, exponent=5.0):
    """Maske in macOS-Icon-Form (Superellipse), kantengeglättet."""
    big = size * scale
    half = big / 2.0
    points = []
    steps = 720
    for i in range(steps):
        t = 2 * math.pi * i / steps
        ct, st = math.cos(t), math.sin(t)
        x = half + half * math.copysign(abs(ct) ** (2 / exponent), ct)
        y = half + half * math.copysign(abs(st) ** (2 / exponent), st)
        points.append((x, y))
    mask = Image.new("L", (big, big), 0)
    ImageDraw.Draw(mask).polygon(points, fill=255)
    return mask.resize((size, size), Image.LANCZOS)


def build_app_icon(work):
    src = os.path.join(RAW, "appicon-c", "appicon-c.png")
    art = Image.open(src).convert("RGBA")
    canvas, body = 1024, 824
    margin = (canvas - body) // 2
    art = art.resize((body, body), Image.LANCZOS)
    mask = squircle_mask(body)

    icon = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    # weicher Schlagschatten wie bei macOS-Icons
    shadow_alpha = Image.new("L", (canvas, canvas), 0)
    shadow_alpha.paste(mask, (margin, margin + 12))
    shadow_alpha = shadow_alpha.filter(ImageFilter.GaussianBlur(14)).point(lambda p: int(p * 0.30))
    shadow = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    shadow.putalpha(shadow_alpha)
    icon = Image.alpha_composite(icon, shadow)

    plate = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    plate.paste(art, (margin, margin), mask)
    icon = Image.alpha_composite(icon, plate)

    os.makedirs(UI, exist_ok=True)
    icon.resize((256, 256), Image.LANCZOS).save(os.path.join(UI, "appicon.png"), optimize=True)

    iconset = os.path.join(work, "Zunder.iconset")
    os.makedirs(iconset, exist_ok=True)
    for base in (16, 32, 128, 256, 512):
        for factor in (1, 2):
            px = base * factor
            suffix = "" if factor == 1 else "@2x"
            icon.resize((px, px), Image.LANCZOS).save(
                os.path.join(iconset, f"icon_{base}x{base}{suffix}.png"), optimize=True
            )
    out = os.path.join(ROOT, "assets", "Zunder.icns")
    subprocess.run(["/usr/bin/iconutil", "-c", "icns", iconset, "-o", out], check=True)
    icon.save(os.path.join(work, "appicon-1024.png"))


def main():
    if len(sys.argv) != 2:
        sys.exit("Aufruf: build_icons.py <arbeitsverzeichnis>")
    work = os.path.abspath(sys.argv[1])
    os.makedirs(work, exist_ok=True)
    names, symbols, report = build_symbols(work)
    write_sprite(symbols)
    write_sheet(names, symbols, work)
    build_app_icon(work)
    for name, w, h, s in report:
        print(f"{name:14s} {w:4d} x {h:4d}  Faktor {s}")
    print(f"{len(names)} Icons -> ui/icons.svg, App-Icon -> assets/Zunder.icns")


if __name__ == "__main__":
    main()
