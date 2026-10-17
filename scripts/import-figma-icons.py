#!/usr/bin/env python3
"""Imports the design's `Icon` component set from Figma into ui/icons/.

usage: import-figma-icons.py <figma-file.json> [token-file]

Renders every variant as SVG through the Figma REST API (two requests),
splits each icon into one SVG per design color, and writes ui/icons.slint:
an `Icons` global with one `IconLayers` value per variant. The `Icon`
component tints each layer from `Theme`, so icons follow the theme.
"""
import json
import pathlib
import re
import sys
import urllib.request
import xml.etree.ElementTree as ET

FILE_KEY = "xmUjmpibweZILUfLWBDXOD"
ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "ui" / "icons"
# Design color -> layer (tinted in ui/icon.slint)
LAYERS = {"#8f8f8f": "fg", "#d37918": "accent", "#ab6314": "dim", "#303030": "line", "#373737": "line"}
DRAWABLE = {"path", "rect", "circle", "ellipse", "line", "polyline", "polygon"}
SVG_NS = "http://www.w3.org/2000/svg"
# The VLC cone is a VideoLAN trademark; the title bar uses Ferricone's icon.
SKIP = {"vlc"}


def slug(variant_name):
    """'name=volume, state=active, value=high, …' -> 'volume-active-high'"""
    props = dict(part.split("=", 1) for part in variant_name.split(", "))
    parts = [props["name"]]
    parts += [props[k] for k in ("state", "value", "direction") if props.get(k, "default") != "default"]
    if props.get("EQ", "default") != "default":
        parts += ["eq", props["EQ"]]
    if props.get("two-tone") == "yes":
        parts.append("two-tone")
    return re.sub(r"[^a-z0-9]+", "-", " ".join(parts).lower()).strip("-")


def fetch(url, token=None):
    req = urllib.request.Request(url, headers={"X-Figma-Token": token} if token else {})
    with urllib.request.urlopen(req, timeout=120) as r:
        return r.read()


def color_of(el):
    for attr in ("fill", "stroke"):
        value = (el.get(attr) or "").lower()
        if value.startswith("#"):
            return value
    return None


def split_layers(svg_bytes):
    """One SVG per layer, keeping only that layer's shapes."""
    ET.register_namespace("", SVG_NS)
    root = ET.fromstring(svg_bytes)
    present = {LAYERS.get(color_of(el)) for el in root.iter() if el.tag.split("}")[-1] in DRAWABLE}
    out = {}
    for layer in sorted(present - {None}):
        tree = ET.fromstring(svg_bytes)
        for parent in list(tree.iter()):
            for el in list(parent):
                if el.tag.split("}")[-1] in DRAWABLE and LAYERS.get(color_of(el)) != layer:
                    parent.remove(el)
        out[layer] = ET.tostring(tree, encoding="unicode")
    return out


def main():
    figma_json = pathlib.Path(sys.argv[1])
    token = pathlib.Path(sys.argv[2] if len(sys.argv) > 2 else "~/.config/figma/token").expanduser().read_text().strip()
    doc = json.loads(figma_json.read_text())
    icon_set = next(c for page in doc["document"]["children"] for c in page["children"]
                    if c["type"] == "COMPONENT_SET" and c["name"] == "Icon")
    # The file has a few duplicate variants (same properties); keep the one
    # the design's frames use most.
    uses = {}
    stack = [doc["document"]]
    while stack:
        node = stack.pop()
        if node.get("componentId"):
            uses[node["componentId"]] = uses.get(node["componentId"], 0) + 1
        stack.extend(node.get("children") or [])
    by_slug = {}
    for v in icon_set["children"]:
        name = slug(v["name"])
        kept = by_slug.get(name)
        if kept is None or uses.get(v["id"], 0) > uses.get(kept, 0):
            if kept is not None:
                print(f"duplicate {name}: dropping {kept}", file=sys.stderr)
            by_slug[name] = v["id"]
        else:
            print(f"duplicate {name}: dropping {v['id']}", file=sys.stderr)
    # A stray variant named "0" can't be a Slint identifier.
    variants = {node_id: name for name, node_id in by_slug.items()
                if name not in SKIP and name[0].isalpha()}

    urls = {}
    ids = list(variants)
    for i in range(0, len(ids), 104):
        query = f"ids={','.join(ids[i:i + 104])}&format=svg&svg_include_id=false&svg_simplify_stroke=true"
        reply = json.loads(fetch(f"https://api.figma.com/v1/images/{FILE_KEY}?{query}", token))
        if reply.get("err"):
            sys.exit(f"Figma: {reply['err']}")
        urls.update(reply["images"])

    OUT.mkdir(parents=True, exist_ok=True)
    for old in OUT.glob("*.svg"):
        old.unlink()
    entries = []
    for node_id, name in sorted(variants.items(), key=lambda kv: kv[1]):
        layers = split_layers(fetch(urls[node_id])) if urls.get(node_id) else {}
        for layer, svg in layers.items():
            (OUT / f"{name}.{layer}.svg").write_text(svg)
        fields = ", ".join(f'{layer}: @image-url("icons/{name}.{layer}.svg")' for layer in sorted(layers))
        value = f": {{ {fields} }}" if fields else ""  # blank icons, such as an unchecked check
        entries.append(f"    out property <IconLayers> {name}{value};")

    (ROOT / "ui" / "icons.slint").write_text(
        "// Generated by scripts/import-figma-icons.py from the design's Icon set. Do not edit.\n"
        "// One image per design color; ui/icon.slint tints each from Theme.\n"
        "export struct IconLayers { fg: image, accent: image, dim: image, line: image }\n\n"
        "export global Icons {\n" + "\n".join(entries) + "\n}\n")
    print(f"{len(entries)} icons, {len(list(OUT.glob('*.svg')))} layer files")


if __name__ == "__main__":
    main()
