"""Translate real openpyxl style objects to Locus's display protocol."""
from __future__ import annotations

from copy import copy
import colorsys

from openpyxl.styles import Alignment, Border, Color, Font, PatternFill, Side
from openpyxl.styles.colors import COLOR_INDEX

COMPONENTS = ("font", "fill", "border", "alignment", "number_format")
EDGES = ("top", "right", "bottom", "left")
SEMANTIC = {"default": "000000", "text": "202020", "secondary": "707070", "accent": "4676AF",
    "success": "387548", "warning": "956D20", "error": "B44040", "surface": "FFFFFF",
    "subtle": "F0F0F0", "accent-soft": "E5ECF5", "success-soft": "E3EFE6", "warning-soft": "F4EDDC",
    "error-soft": "F5E5E5", "border": "D0D0D0", "transparent": "FFFFFF"}
THEME = ("FFFFFF", "000000", "EEECE1", "1F497D", "4F81BD", "C0504D", "9BBB59", "8064A2", "4BACC6", "F79646", "0000FF", "800080")


def from_color(value: str) -> Color:
    if value == "default": return Color(auto=True)
    if value in SEMANTIC: return Color(rgb=SEMANTIC[value])
    value = value.lstrip("#")
    if len(value) in (3, 4): value = "".join(ch * 2 for ch in value)
    return Color(rgb=value[:6])


def to_color(value, fallback: str, previous: str | None = None) -> str:
    if value is None: return fallback
    if previous is not None and value == from_color(previous): return previous
    if value.type == "auto": return fallback
    if value.type == "rgb": rgb = value.rgb[-6:]
    elif value.type == "indexed":
        if value.indexed >= len(COLOR_INDEX): return fallback
        rgb = COLOR_INDEX[value.indexed][-6:]
    elif value.type == "theme":
        if not 0 <= value.theme < len(THEME): raise ValueError("Invalid Excel theme color")
        rgb = THEME[value.theme]
    else: raise ValueError("Unsupported Excel color")
    if value.tint:
        r, g, b = (int(rgb[i:i + 2], 16) / 255 for i in (0, 2, 4))
        h, light, sat = colorsys.rgb_to_hls(r, g, b)
        light = light * (1 + value.tint) if value.tint < 0 else light * (1 - value.tint) + value.tint
        rgb = "".join(f"{round(n * 255):02X}" for n in colorsys.hls_to_rgb(h, light, sat))
    return "#" + rgb.upper()


def encode_component(value, name: str, previous=None):
    old = previous or {}
    if name == "font":
        if any(getattr(value, attr, False) for attr in ("outline", "shadow", "condense", "extend")):
            raise NotImplementedError("CSV does not support outline/shadow/condensed/extended fonts")
        return {"name": value.name or "ui", "size": float(value.sz or 9.75), "bold": bool(value.b),
            "italic": bool(value.i), "strike": bool(value.strike), "underline": value.u or "none",
            "color": to_color(value.color, "default", old.get("color")), "vertAlign": value.vertAlign or "baseline"}
    if name == "fill":
        if not isinstance(value, PatternFill): raise NotImplementedError("CSV supports PatternFill; GradientFill is not supported")
        return {"patternType": value.patternType or "none", "fgColor": to_color(value.fgColor, "transparent", old.get("fgColor")),
            "bgColor": to_color(value.bgColor, "transparent", old.get("bgColor"))}
    if name == "border":
        for edge in ("diagonal", "start", "end", "vertical", "horizontal"):
            if getattr(value, edge, None) is not None and getattr(value, edge).style:
                raise NotImplementedError(f"CSV does not support the {edge} border")
        return {edge: {"style": getattr(value, edge).style or "none", "color": to_color(getattr(value, edge).color, "border", old.get(edge, {}).get("color"))}
            if getattr(value, edge) is not None else {"style": "none", "color": "border"} for edge in EDGES}
    if name == "alignment":
        if value.horizontal in {"fill", "centerContinuous"} or value.relativeIndent or value.justifyLastLine:
            raise NotImplementedError("CSV supports standard alignment, wrap, indent, rotation and shrink_to_fit")
        if int(value.indent) != value.indent: raise ValueError("CSV alignment indent must be an integer")
        return {"horizontal": value.horizontal or "general", "vertical": value.vertical or "bottom",
            "wrapText": bool(value.wrapText), "shrinkToFit": bool(value.shrinkToFit),
            "textRotation": value.textRotation or 0, "indent": int(value.indent or 0), "readingOrder": int(value.readingOrder or 0)}
    if name == "number_format": return value or "General"
    raise ValueError(name)


def decode_component(value, name: str):
    if name == "font":
        return Font(name=value["name"], sz=value["size"], b=value["bold"], i=value["italic"], strike=value["strike"],
            u=None if value["underline"] == "none" else value["underline"], color=from_color(value["color"]),
            vertAlign=None if value["vertAlign"] == "baseline" else value["vertAlign"])
    if name == "fill":
        return PatternFill(patternType=None if value["patternType"] == "none" else value["patternType"],
            fgColor=from_color(value["fgColor"]), bgColor=from_color(value["bgColor"]))
    if name == "border":
        return Border(**{edge: Side(style=None if side["style"] == "none" else side["style"], color=from_color(side["color"])) for edge, side in value.items()})
    if name == "alignment":
        return Alignment(horizontal=value["horizontal"], vertical=value["vertical"], wrap_text=value["wrapText"],
            shrink_to_fit=value["shrinkToFit"], text_rotation=value["textRotation"], indent=value["indent"], readingOrder=value["readingOrder"])
    return value


def apply_style(cell, style):
    font = copy(cell.font)
    for name, attr in (("font", "name"), ("size", "sz"), ("bold", "b"), ("color", "color")):
        if name in style:
            setattr(font, attr, from_color(style[name]) if name == "color" else style[name] * 72 / 96 if name == "size" else style[name])
    cell.font = font
    if "background" in style: cell.fill = PatternFill("solid", fgColor=from_color(style["background"]))
    if "border" in style:
        b = style["border"]
        border = copy(cell.border)
        kind = b.get("style", "solid")
        kind = "none" if b.get("width") == 0 else {"solid": "thick" if b.get("width", 1) >= 3 else "medium" if b.get("width", 1) == 2 else "thin"}.get(kind, kind)
        for edge in b.get("edges", EDGES): setattr(border, edge, Side(style=None if kind == "none" else kind, color=from_color(b.get("color", "border"))))
        cell.border = border
    for name, value in style.get("excel", {}).items():
        attr = "number_format" if name == "numberFormat" else name
        setattr(cell, attr, decode_component(value, attr))


def components(cell):
    return {name: copy(getattr(cell, name)) for name in COMPONENTS}
