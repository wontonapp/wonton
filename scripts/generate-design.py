#!/usr/bin/env python3
"""从 design/ 生成原生令牌、图标、噪点和客户端网页令牌映射。--check 只比对，不一致时失败。

其余用法见 design/README.md。"""

# 这个脚本不在核心产物的键里：它只改生成的界面代码，不碰 Rust 核心（见 .github/workflows/README.md 的“核心产物”）。

import base64
import copy
import hashlib
import json
import math
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import urllib.request
import zipfile
from xml.etree import ElementTree
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DESIGN = ROOT / "design"
IOS = ROOT / "apps/ios/Wonton"
COMPOSE = ROOT / "apps/compose/shared/src/androidMain"
KOTLIN = COMPOSE / "kotlin/app/wonton/ui"
WEB_GENERATED = ROOT / "crates/wonton-client/src/pages/bridge/generated"
GENERATED_DIRS = [
    IOS / "Design/Generated",
    IOS / "Assets.xcassets/IconsColor",
    IOS / "Assets.xcassets/IconsMono",
    KOTLIN / "design/generated",
    KOTLIN / "icons",
    WEB_GENERATED,
]

VALKYRIE_VERSION = "1.2.1"
VALKYRIE_URL = (
    f"https://github.com/ComposeGears/Valkyrie/releases/download/cli-{VALKYRIE_VERSION}/"
    f"valkyrie-cli-{VALKYRIE_VERSION}.zip"
)
VALKYRIE_SHA256 = "5929fb15395059e566c51dcdb2bed98a2d2e92ad8d344f8186d3b701be6533b3"

KUBAI = DESIGN / "kubai"
# 浅色、深色下的页面底和卡片底：crates/wonton-client/src/appearance/derive.rs 的 canvas 和 surfaceSubtle。
ICON_BACKGROUNDS = {"light": ["#FFFFFF", "#F4F4F4"], "dark": ["#0E0E0E", "#262626"]}
# WCAG 1.4.11 对图形的最低要求。
ICON_MIN_CONTRAST = 3.0
# OKLab 彩度小于它的是白、黑、灰，换色时不动。按彩度不按通道差：角标的深灰底（#33363D）通道差有 10，
# 当成主色重染后在深色页面上只剩 1.4:1。上游最淡的浅面彩度约 0.04。
KUBAI_GRAY_CHROMA = 0.02
# 上游的十六进制颜色是四舍五入过的，按主色混白混黑反推时留这点误差（0–255）。
KUBAI_MIX_TOLERANCE = 2
SVG_HEX = re.compile(r'(?<==")#([0-9A-Fa-f]{6}|[0-9A-Fa-f]{3})(?=")')
SVG_NAMESPACE = "http://www.w3.org/2000/svg"
SVG_TAG = "{" + SVG_NAMESPACE + "}"
SVG_SHAPE_ONLY = {SVG_TAG + "mask", SVG_TAG + "clipPath"}
SVG_HEX_VALUE = re.compile(r"#([0-9A-Fa-f]{6}|[0-9A-Fa-f]{3})")
SVG_OPACITY = {"fill": "fill-opacity", "stroke": "stroke-opacity", "stop-color": "stop-opacity"}
# 单色版：OKLab 亮度不比主色高出这么多的算实色；通道都不低于 KUBAI_WHITE 的灰算白。
KUBAI_INK_MARGIN = 0.02
KUBAI_WHITE = 240
# 浅面的不透明度：FLOOR 起，含主色越多越浓。要比挖空的地方明显深、又比实色的描边明显浅，
# 取值是看着 24pt 和 14pt 的渲染调的。
KUBAI_TINT_FLOOR = 0.25
KUBAI_TINT_SCALE = 0.4
# 深色页面上浅面里混的白压到这么多：原样的浅面在黑底上太亮，比旁边的字还抢眼。
KUBAI_DARK_WHITE = 0.8
# 去掉底的图形绕画布中心（15.5, 12.5）放大：上游的线条是按放在圆底里画的，单独用偏小。
KUBAI_BARE_TRANSFORM = "translate(15.5 12.5) scale(1.5) translate(-15.5 -12.5)"
# 蒙版的范围照上游自己的蒙版写：比画布（-2.5 -5.5 36 36）大一圈。
KUBAI_MASK_BOX = {"x": "-40", "y": "-40", "width": "111", "height": "109"}
SVG_MATRIX = re.compile(r'values="([^"]+)"')
SVG_PAINTS = {"fill", "stroke", "stop-color", "flood-color"}
# 只认 name="value" 这一种写法：换色按它找颜色，别的写法（单引号、等号两边有空格）会被漏掉。
SVG_LOOSE_ATTRIBUTE = re.compile(r"""[\w:-]\s*=\s*'|[\w:-]\s+=|=\s+["']""")
SVG_CSS = re.compile(r"var\(|color-mix\(|calc\(|currentColor|url\((?!#)", re.I)
SVG_PAINT = re.compile(r"#[0-9A-Fa-f]{6}|#[0-9A-Fa-f]{3}|url\(#[\w-]+\)")
SVG_ELEMENTS = {
    "svg", "defs", "g", "path", "rect", "circle", "ellipse", "text", "linearGradient", "stop", "mask", "clipPath",
    "filter", "feFlood", "feColorMatrix", "feOffset", "feGaussianBlur", "feComposite", "feBlend",
}
SVG_ATTRIBUTES = {
    "xmlns", "viewBox", "width", "height", "id", "x", "y", "x1", "y1", "x2", "y2", "cx", "cy", "r", "rx", "ry", "d",
    "transform", "opacity", "fill", "fill-opacity", "fill-rule", "clip-path", "clip-rule", "mask", "maskUnits",
    "filter", "stroke", "stroke-width", "stroke-opacity", "stroke-linecap", "stroke-linejoin", "stroke-dasharray",
    "stroke-dashoffset", "pathLength", "offset", "stop-color", "stop-opacity", "gradientUnits", "color-interpolation-filters",
    "flood-opacity", "in", "in2", "result", "type", "values", "dy", "stdDeviation", "operator", "k2", "k3", "mode",
    "text-anchor", "font-size", "font-weight", "font-family",
}

BANNER = "// 由 scripts/generate-design.py 从 design/ 生成，不要手改。\n"
XCASSETS_INFO = {"info": {"author": "xcode", "version": 1}}


def camel_upper(name):
    return name[0].upper() + name[1:]


def for_platform(tokens, platform):
    """圆角可以分平台（ADR-0029）。"""
    radii = {name: value[platform] if isinstance(value, dict) else value for name, value in tokens["radii"].items()}
    return {**tokens, "radii": radii}


def swift_tokens(tokens):
    enums = []
    for section in ["spacing", "radii", "size", "ratio", "gesture"]:
        lines = [f"    static let {name}: CGFloat = {value}" for name, value in tokens[section].items()]
        enums.append(f"enum Wonton{camel_upper(section)} {{\n" + "\n".join(lines) + "\n}\n")
    motion = [f"    static let {name}: Double = {value / 1000}" for name, value in tokens["motion"].items()]
    enums.append("enum WontonMotion {\n" + "\n".join(motion) + "\n}\n")
    styles = [
        f"    static let {name} = WontonTextStyle("
        f'size: {s["size"]}, line: {s["line"]}, weight: .{s["weight"]}, relativeTo: .{s["relativeTo"]})'
        for name, s in tokens["typography"].items()
    ]
    enums.append("extension WontonTextStyle {\n" + "\n".join(styles) + "\n}\n")
    return BANNER + "import SwiftUI\n\n" + "\n".join(enums)


def kotlin_tokens(tokens):
    def section(name, values, render):
        lines = "\n".join(f"    val {key} = {render(value)}" for key, value in values.items())
        return f"object Wonton{camel_upper(name)} {{\n{lines}\n}}\n"

    weights = {"bold": "Bold", "semibold": "SemiBold", "medium": "Medium", "regular": "Normal"}
    # 是类不是 object：字体是用户选的，运行时才知道（ADR-0028）。
    styles = "\n".join(
        f'    val {name} = TextStyle(fontFamily = fontFamily, fontSize = {s["size"]}.sp, lineHeight = {s["line"]}.sp, '
        f'fontWeight = FontWeight.{weights[s["weight"]]})'
        for name, s in tokens["typography"].items()
    )
    parts = [
        section("spacing", tokens["spacing"], lambda v: f"{v}.dp"),
        section("radii", tokens["radii"], lambda v: f"{v}.dp"),
        section("size", tokens["size"], lambda v: f"{v}.dp"),
        section("ratio", tokens["ratio"], lambda v: f"{v}f"),
        section("gesture", tokens["gesture"], lambda v: f"{v}f"),
        section("motion", tokens["motion"], str),
        f"class WontonTypography(fontFamily: FontFamily?) {{\n{styles}\n}}\n",
    ]
    return (
        BANNER
        + "@file:Suppress(\"MagicNumber\")\n\npackage app.wonton.ui.design.generated\n\n"
        + "import androidx.compose.ui.text.TextStyle\n"
        + "import androidx.compose.ui.text.font.FontFamily\n"
        + "import androidx.compose.ui.text.font.FontWeight\n"
        + "import androidx.compose.ui.unit.dp\n"
        + "import androidx.compose.ui.unit.sp\n\n"
        + "\n".join(parts)
    )


def rust_web_tokens(tokens):
    colors = []
    radii = []
    fonts = []
    platforms = {platform: for_platform(tokens, platform) for platform in ("ios", "android")}
    for variable, source in tokens["web_variables"].items():
        if len(source) != 1:
            raise ValueError(f"网页变量必须只有一个来源：{variable}")
        kind, name = next(iter(source.items()))
        if kind == "color":
            colors.append(f'    ("{variable}", |c| c.{name}),')
        elif kind == "radius":
            ios, android = (float(platforms[platform]["radii"][name]) for platform in ("ios", "android"))
            radii.append(f'    ("{variable}", {ios}, {android}),')
        elif kind == "font" and name == "font":
            fonts.append(variable)
        else:
            raise ValueError(f"未知网页变量来源：{variable} = {source}")
    if len(fonts) != 1:
        raise ValueError("网页变量必须声明一个 AppearanceViewModel 字体来源")
    return (
        BANNER
        + "use wonton_protocol::{Rgb, ThemeColors};\n\n"
        + "type ColorVariable = (&'static str, fn(&ThemeColors) -> Rgb);\n\n"
        + "pub(crate) const COLOR_VARIABLES: &[ColorVariable] = &[\n"
        + "\n".join(colors)
        + "\n];\n\n"
        + "pub(crate) const RADIUS_VARIABLES: &[(&str, f64, f64)] = &[\n"
        + "\n".join(radii)
        + "\n];\n\n"
        + f'pub(crate) const FONT_VARIABLE: &str = "{fonts[0]}";\n'
    )


def icon_variants(icons):
    """(文件名里的语义名, 语义条目, 样式)。"""
    for name, icon in icons["icons"].items():
        yield name, icon, "regular"
        if icon.get("fill"):
            yield f"{name}Fill", icon, "fill"


def upstream_svg(icon, style):
    file = f'{icon["phosphor"]}-fill.svg' if style == "fill" else f'{icon["phosphor"]}.svg'
    return DESIGN / "phosphor" / style / file


def swift_icons(icons):
    """iOS 只用 Kubai：没写 kubai 的语义名不进枚举。实心版的名字留着（界面按选中态取名），指向同一张图。"""
    cases, assets = [], []
    for variant, icon, _ in icon_variants(icons):
        if "kubai" not in icon:
            continue
        name = variant.removesuffix("Fill") if icon.get("fill") else variant
        cases.append(f"    case {variant}")
        assets.append(f'        case .{variant}: "{name}"')
    return (
        BANNER
        + "enum WontonIcon: String {\n" + "\n".join(cases) + "\n}\n\n"
        + "extension WontonIcon {\n"
        + "    /// 资源名的前半截：彩色版是它加 `Color`，单色版加 `Mono`。\n"
        + "    var assetName: String {\n        switch self {\n" + "\n".join(assets) + "\n        }\n    }\n}\n"
    )


def ios_assets(icons):
    """每个图标两套：彩色分层（原色，浅色、深色各一张）和单色（模板渲染，颜色跟界面走）。都保留矢量。"""
    files = {
        IOS / "Assets.xcassets/Contents.json": json_bytes(XCASSETS_INFO),
        IOS / "Assets.xcassets/IconsColor/Contents.json": json_bytes(XCASSETS_INFO),
        IOS / "Assets.xcassets/IconsMono/Contents.json": json_bytes(XCASSETS_INFO),
    }
    check_palette(icons["palette"])
    check_kubai_files(KUBAI, kubai_used(icons))
    base = hex_rgb(icons["upstream"]["kubai"]["base"])
    for name, icon in icons["icons"].items():
        if "kubai" not in icon:
            continue
        source = (KUBAI / f'svg/{icon["kubai"]}.svg').read_text()
        if icon.get("bare"):
            source = kubai_bare(source, icons["upstream"]["kubai"]["base"])
        color = icons["palette"][icon["color"]]
        files.update(image_set(name, "Color", "original", icon, {
            "": kubai_recolored(source, base, hex_rgb(color["light"]), 1),
            "Dark": kubai_recolored(source, base, hex_rgb(color["dark"]), KUBAI_DARK_WHITE),
        }))
        files.update(image_set(name, "Mono", "template", icon, {"": kubai_mono(icon["kubai"], source, base)}))
    return files


def image_set(name, suffix, intent, icon, variants):
    """variants：外观后缀（空是通用，Dark 是深色）到 SVG。"""
    folder = IOS / f"Assets.xcassets/Icons{suffix}/{name}{suffix}.imageset"
    files, images = {}, []
    for appearance, svg in variants.items():
        check_plain_svg(f"{name}{suffix}{appearance}", svg)
        image = {"filename": f"{name}{suffix}{appearance}.svg", "idiom": "universal"}
        if appearance == "Dark":
            image["appearances"] = [{"appearance": "luminosity", "value": "dark"}]
        if icon.get("mirror"):
            image["language-direction"] = "left-to-right"
        images.append(image)
        files[folder / image["filename"]] = svg.encode()
    contents = {
        "images": images,
        **XCASSETS_INFO,
        "properties": {"preserves-vector-representation": True, "template-rendering-intent": intent},
    }
    files[folder / "Contents.json"] = json_bytes(contents)
    return files


def hex_rgb(value):
    return tuple(int(value[i:i + 2], 16) for i in (1, 3, 5))


def rgb_hex(color):
    return "#%02X%02X%02X" % tuple(max(0, min(255, round(channel))) for channel in color)


def linear(channel):
    channel /= 255
    return channel / 12.92 if channel <= 0.04045 else ((channel + 0.055) / 1.055) ** 2.4


def contrast(first, second):
    """WCAG 的对比度。"""
    lights = sorted(
        (0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b) for r, g, b in (first, second)), reverse=True
    )
    return (lights[0] + 0.05) / (lights[1] + 0.05)


def check_palette(palette):
    for name, values in palette.items():
        for appearance, backgrounds in ICON_BACKGROUNDS.items():
            for background in backgrounds:
                ratio = contrast(hex_rgb(values[appearance]), hex_rgb(background))
                if ratio < ICON_MIN_CONTRAST:
                    sys.exit(
                        f"图标颜色 {name} {values[appearance]} 在 {background} 上的对比度只有 {ratio:.2f}，"
                        f"要 {ICON_MIN_CONTRAST}"
                    )


def oklab(color):
    r, g, b = (linear(channel) for channel in color)
    l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b) ** (1 / 3)
    m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b) ** (1 / 3)
    s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b) ** (1 / 3)
    return (
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    )


def from_oklab(lab):
    lightness, a, b = lab
    l = (lightness + 0.3963377774 * a + 0.2158037573 * b) ** 3
    m = (lightness - 0.1055613458 * a - 0.0638541728 * b) ** 3
    s = (lightness - 0.0894841775 * a - 1.2914855480 * b) ** 3
    channels = (
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    )
    return tuple(
        255 * (12.92 * c if c <= 0.0031308 else 1.055 * max(c, 0) ** (1 / 2.4) - 0.055) for c in channels
    )


def mix_weights(color, base):
    """把 color 写成 a×主色 + b×白 + 剩下的黑（最小二乘），返回 (a, b, 最大通道误差)。"""
    bb = sum(x * x for x in base)
    ww = 3 * 255 * 255
    bw = 255 * sum(base)
    cb = sum(x * y for x, y in zip(color, base))
    cw = 255 * sum(color)
    det = bb * ww - bw * bw
    a, b = (cb * ww - cw * bw) / det, (cw * bb - cb * bw) / det
    if a < 0:
        a, b = 0, cw / ww
    if b < 0:
        a, b = cb / bb, 0
    error = max(abs(a * x + b * 255 - y) for x, y in zip(base, color))
    return a, b, error


def kubai_recolor(color, base, accent, white=1):
    """上游的颜色都是从主色调出来的：多数是主色混白或混黑，深色描边是主色在 OKLab 里压暗并偏一点色相。
    前一种按同样的配比换成新主色；后一种配比对不上，在 OKLCH 里照搬亮度比、彩度比和色相差。
    white 小于 1 时混进去的白压暗这么多（深色页面上用）；纯白的细节不算混色，不受影响。"""
    if is_gray(color):
        return color
    a, b, error = mix_weights(color, base)
    if error <= KUBAI_MIX_TOLERANCE:
        return tuple(a * x + b * white * 255 for x in accent)
    (cl, ca, cb), (bl, ba, bb), (al, aa, ab) = oklab(color), oklab(base), oklab(accent)
    chroma = math.hypot(aa, ab) * math.hypot(ca, cb) / math.hypot(ba, bb)
    hue = math.atan2(ab, aa) + math.atan2(cb, ca) - math.atan2(bb, ba)
    return from_oklab((al * cl / bl, chroma * math.cos(hue), chroma * math.sin(hue)))


def kubai_recolored(svg, base, accent, white):
    def paint(match):
        value = match.group(1)
        value = value if len(value) == 6 else "".join(c * 2 for c in value)
        return rgb_hex(kubai_recolor(hex_rgb("#" + value), base, accent, white))

    def matrix(match):
        # 投影和高光的颜色写在 feColorMatrix 第 5 列（每行的常数项，0–1）。
        values = match.group(1).split()
        color = kubai_recolor(tuple(float(values[i]) * 255 for i in (4, 9, 14)), base, accent, white)
        for index, channel in zip((4, 9, 14), color):
            values[index] = f"{max(0, min(255, channel)) / 255:.4f}".rstrip("0").rstrip(".")
        return f'values="{" ".join(values)}"'

    svg = SVG_HEX.sub(paint, svg)
    return SVG_MATRIX.sub(matrix, svg)


def is_gray(color):
    _, a, b = oklab(color)
    return math.hypot(a, b) < KUBAI_GRAY_CHROMA


def is_white(value):
    if not SVG_HEX_VALUE.fullmatch(value or ""):
        return False
    color = hex_rgb(long_hex(value))
    return is_gray(color) and min(color) >= KUBAI_WHITE


def long_hex(value):
    return value if len(value) == 7 else "#" + "".join(c * 2 for c in value[1:])


def kubai_ink(color, base):
    """单色版里这个颜色画多浓：主色、深色描边和黑的角标是实的；主色混白的浅面按含主色的多少变淡。"""
    if oklab(color)[0] <= oklab(base)[0] + KUBAI_INK_MARGIN:
        return 1.0
    share, _, _ = mix_weights(color, base)
    return round(KUBAI_TINT_FLOOR + KUBAI_TINT_SCALE * max(0.0, min(1.0, share)), 2)


def painted(element):
    """element 底下带颜色属性的节点；内嵌的蒙版和裁剪路径里的黑白是形状不是颜色，跳过。"""
    if element.tag in SVG_SHAPE_ONLY:
        return
    yield element
    for child in element:
        yield from painted(child)


def white_shapes(element):
    """element 里白色的部分，画成黑的（蒙版里黑是挖掉）；没有白色返回 None。"""
    shape = copy.deepcopy(element)
    found = False
    for node in painted(shape):
        for attribute in ("fill", "stroke"):
            if attribute not in node.attrib:
                continue
            white = is_white(node.get(attribute))
            found = found or white
            node.set(attribute, "#000" if white else "none")
    return shape if found else None


def knock_out(children, masks, name):
    """白色在彩色版里是盖在上面的纸面和字；模板渲染只看透明度，盖不住下面，所以改成把先画的东西挖空。
    按绘制顺序走：碰到带白色的元素，就给它前面的兄弟套一层蒙版，蒙版里是它的白色部分。"""
    drawn, open_mask = [], None
    for child in children:
        cut = white_shapes(child)
        if child.tag == SVG_TAG + "g":
            child[:] = knock_out(list(child), masks, name)
        for node in painted(child):
            for attribute in ("fill", "stroke"):
                if is_white(node.get(attribute)):
                    node.set(attribute, "none")
        if cut is not None and drawn:
            if open_mask is None:
                open_mask = ElementTree.Element(
                    SVG_TAG + "mask", {"id": f"ko-{name}-{len(masks)}", "maskUnits": "userSpaceOnUse", **KUBAI_MASK_BOX}
                )
                ElementTree.SubElement(open_mask, SVG_TAG + "rect", {**KUBAI_MASK_BOX, "fill": "#fff"})
                masks.append(open_mask)
                group = ElementTree.Element(SVG_TAG + "g", {"mask": f"url(#{open_mask.get('id')})"})
                group.extend(drawn)
                drawn = [group]
            open_mask.append(cut)
        drawn.append(child)
        # 只有纯白的单个图形挖完就不再画东西，下一个白色可以并进同一层蒙版；别的元素可能自己画了东西，
        # 后面的白色要连它一起挖，另开一层。
        if cut is None or len(child) or child.get("fill") != "none" or child.get("stroke", "none") != "none":
            open_mask = None
    return drawn


def kubai_bare(svg, base):
    """只留图形本体：去掉闭合的底（圆底和它的描边、高光），剩下的线条放大。用在本来就放进圆钮或成排出现的
    箭头和加号上，免得圆里套圆。线条一律改成主色：白线没有底可衬，深色描边单独放在深色页面上不到 3:1。"""
    ElementTree.register_namespace("", SVG_NAMESPACE)
    root = ElementTree.fromstring(svg)
    glyph = ElementTree.Element(SVG_TAG + "g", {"transform": KUBAI_BARE_TRANSFORM})
    # 只看画出来的图形：defs、蒙版和裁剪路径里的 path 是形状定义，不是线条。
    visible = [node for child in root if child.tag != SVG_TAG + "defs" for node in painted(child)]
    for node in visible:
        if node.tag != SVG_TAG + "path" or "d" not in node.attrib:
            continue
        if node.get("d").rstrip().lower().endswith("z") or "mask" in node.attrib or "filter" in node.attrib:
            continue
        node.set("stroke", base)
        glyph.append(node)
    if not len(glyph):
        sys.exit("去掉底以后什么都不剩，这个图标不能写 bare")
    for child in list(root):
        root.remove(child)
    root.append(glyph)
    return ElementTree.tostring(root, encoding="unicode")


def kubai_mono(name, svg, base):
    """单色版仍是 Kubai 的图形，交给模板渲染上色：颜色全换成黑，浓淡写进不透明度，白色挖空。
    投影和高光的滤镜去掉：模板渲染会把投影也染成前景色，小图标周围糊一圈。"""
    ElementTree.register_namespace("", SVG_NAMESPACE)
    root = ElementTree.fromstring(svg)
    defs = root.find(SVG_TAG + "defs")
    if defs is None:
        defs = ElementTree.Element(SVG_TAG + "defs")
        root.insert(0, defs)
    for node in defs.findall(SVG_TAG + "filter"):
        defs.remove(node)
    for node in root.iter():
        node.attrib.pop("filter", None)
    body = [child for child in root if child is not defs]
    for child in body:
        root.remove(child)
    masks = []
    root.extend(knock_out(body, masks, name))
    # 蒙版里的黑白是形状不是颜色，不动。
    defs.extend(masks)
    in_mask = {id(node) for tag in SVG_SHAPE_ONLY for shape in root.iter(tag) for node in shape.iter()}
    for node in root.iter():
        if id(node) in in_mask:
            continue
        for attribute, opacity in SVG_OPACITY.items():
            value = node.get(attribute, "")
            if not SVG_HEX_VALUE.fullmatch(value):
                continue
            ink = kubai_ink(hex_rgb(long_hex(value)), base)
            node.set(attribute, "#000000")
            if ink < 1:
                node.set(opacity, str(round(ink * float(node.get(opacity, "1")), 2)))
    return ElementTree.tostring(root, encoding="unicode")


def check_plain_svg(name, svg):
    """CoreSVG 不认 CSS：颜色没展开成具体值的（color-mix()、var()、currentColor、style）会画成纯黑。"""
    problems = sorted(set(SVG_LOOSE_ATTRIBUTE.findall(svg)))
    try:
        root = ElementTree.fromstring(svg)
    except ElementTree.ParseError as error:
        sys.exit(f"图标 {name} 不是合法的 SVG：{error}")
    for node in root.iter():
        tag = node.tag.removeprefix(SVG_TAG)
        if tag not in SVG_ELEMENTS:
            problems.append(f"<{tag}>")
        for attribute, value in node.attrib.items():
            if attribute not in SVG_ATTRIBUTES:
                problems.append(attribute)
            elif SVG_CSS.search(value):
                problems.append(f'{attribute}="{value}"')
            elif attribute in SVG_PAINTS and value != "none" and not SVG_PAINT.fullmatch(value):
                problems.append(f'{attribute}="{value}"')
    if problems:
        sys.exit(f"图标 {name} 里有 CoreSVG 画不了的内容：{sorted(set(problems))}")


def kubai_sums(folder):
    lines = (folder / "SHA256SUMS").read_text().splitlines()
    return {path: digest for digest, path in (line.split("  ", 1) for line in lines)}


def check_kubai_files(folder, used):
    """上游原件不手改：登记的每个文件（含 LICENSE）核对哈希，目录里多出来、少了的都报错。"""
    sums = kubai_sums(folder)
    if used != set(sums):
        sys.exit(f"design/kubai 里登记的文件和 icons.toml 用到的对不上：{sorted(used ^ set(sums))}")
    present = {str(path.relative_to(folder)) for path in folder.rglob("*") if path.is_file()} - {"SHA256SUMS"}
    if present != set(sums):
        sys.exit(f"design/kubai 里的文件和 SHA256SUMS 登记的对不上：{sorted(present ^ set(sums))}")
    for path, digest in sums.items():
        if hashlib.sha256((folder / path).read_bytes()).hexdigest() != digest:
            sys.exit(f"design/kubai/{path} 和 SHA256SUMS 对不上：上游原件不手改")


def kubai_used(icons):
    return {f'svg/{icon["kubai"]}.svg' for icon in icons["icons"].values() if "kubai" in icon} | {"LICENSE"}


def vendor_kubai(tarball, icons, folder):
    """从校验过的 npm tarball 里取出 icons.toml 用到的 SVG 和 LICENSE，重写 folder。
    先取到临时目录，全部取到了再换：中途失败时原来的目录不动。"""
    upstream = icons["upstream"]["kubai"]
    data = Path(tarball).read_bytes()
    algorithm, expected = upstream["integrity"].split("-", 1)
    if base64.b64encode(hashlib.new(algorithm, data).digest()).decode() != expected:
        sys.exit(f"{tarball} 的校验值和 icons.toml 的 integrity 对不上")
    wanted = sorted(kubai_used(icons))
    with tempfile.TemporaryDirectory(dir=folder.parent) as temp, tarfile.open(tarball) as archive:
        staged = Path(temp) / "kubai"
        sums = []
        for path in wanted:
            try:
                member = archive.extractfile(f"package/{path}")
            except KeyError:
                member = None
            if member is None:
                sys.exit(f"{upstream['package']} {upstream['version']} 里没有 {path}")
            content = member.read()
            (staged / path).parent.mkdir(parents=True, exist_ok=True)
            (staged / path).write_bytes(content)
            sums.append(f"{hashlib.sha256(content).hexdigest()}  {path}")
        (staged / "SHA256SUMS").write_text("\n".join(sums) + "\n")
        if folder.exists():
            shutil.rmtree(folder)
        staged.rename(folder)
    print(f"design/kubai：{len(wanted)} 个文件")


def json_bytes(value):
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def valkyrie_jar():
    cache = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / f"wonton/valkyrie-{VALKYRIE_VERSION}"
    jar = cache / f"lib/valkyrie-{VALKYRIE_VERSION}-all.jar"
    if jar.exists():
        return jar
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / "valkyrie.zip"
    with urllib.request.urlopen(VALKYRIE_URL) as response:
        archive.write_bytes(response.read())
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    if digest != VALKYRIE_SHA256:
        archive.unlink()
        sys.exit(f"Valkyrie 的哈希不对：{digest}")
    with zipfile.ZipFile(archive) as zipped:
        zipped.extractall(cache)
    archive.unlink()
    return jar


def compose_icons(icons):
    """每个图标一个文件。mirror 是整次运行的开关，所以按是否翻转分两次运行。"""
    jar = valkyrie_jar()
    files = {}
    with tempfile.TemporaryDirectory() as temp:
        temp = Path(temp)
        for mirror in (False, True):
            source, target = temp / f"in-{mirror}", temp / f"out-{mirror}"
            source.mkdir()
            target.mkdir()
            for name, icon, style in icon_variants(icons):
                if bool(icon.get("mirror")) == mirror:
                    shutil.copy(upstream_svg(icon, style), source / f"{name}.svg")
            if not any(source.iterdir()):
                continue
            subprocess.run(
                ["java", "-jar", str(jar), "svgxml2imagevector",
                 f"--input-path={source}", f"--output-path={target}",
                 "--package-name=app.wonton.ui.icons", "--iconpack-name=WontonIcon",
                 "--output-format=lazy-property", "--use-path-data-string=true",
                 "--suppress-unused-receiver=true", f"--auto-mirror={str(mirror).lower()}"],
                check=True, capture_output=True,
            )
            for path in target.rglob("*.kt"):
                files[KOTLIN / "icons" / path.name] = path.read_bytes()
    pack = BANNER + "package app.wonton.ui.icons\n\nobject WontonIcon\n"
    files[KOTLIN / "icons/WontonIcon.kt"] = pack.encode()
    return files


def expected_files():
    tokens = tomllib.loads((DESIGN / "tokens.toml").read_text())
    icons = tomllib.loads((DESIGN / "icons.toml").read_text())
    noise = (DESIGN / "noise.png").read_bytes()
    files = {
        IOS / "Design/Generated/WontonTokens.swift": swift_tokens(for_platform(tokens, "ios")).encode(),
        IOS / "Design/Generated/WontonIcon.swift": swift_icons(icons).encode(),
        IOS / "noise.png": noise,
        COMPOSE / "res/drawable-nodpi/wonton_noise.png": noise,
        KOTLIN / "design/generated/WontonTokens.kt": kotlin_tokens(for_platform(tokens, "android")).encode(),
        WEB_GENERATED / "tokens.rs": rust_web_tokens(tokens).encode(),
    }
    files.update(ios_assets(icons))
    files.update(compose_icons(icons))
    return files


def existing_generated_files():
    return {path for folder in GENERATED_DIRS if folder.exists() for path in folder.rglob("*") if path.is_file()}


def main():
    if "--vendor-kubai" in sys.argv[1:]:
        icons = tomllib.loads((DESIGN / "icons.toml").read_text())
        vendor_kubai(sys.argv[sys.argv.index("--vendor-kubai") + 1], icons, KUBAI)
        return
    check = "--check" in sys.argv[1:]
    files = expected_files()
    stale = existing_generated_files() - set(files)
    differing = [path for path, content in files.items() if not path.exists() or path.read_bytes() != content]
    if check:
        problems = sorted(differing) + sorted(stale)
        if problems:
            names = "\n".join(str(path.relative_to(ROOT)) for path in problems)
            sys.exit(f"生成结果和 design/ 不一致，运行 python3 scripts/generate-design.py：\n{names}")
        print("设计生成结果一致")
        return
    for path in stale:
        path.unlink()
    for path in differing:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(files[path])
    print(f"写入 {len(differing)} 个文件，删除 {len(stale)} 个")


if __name__ == "__main__":
    main()
