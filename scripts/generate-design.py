#!/usr/bin/env python3
"""从 design/ 生成原生令牌、图标、噪点和客户端网页令牌映射。--check 只比对，不一致时失败。

--vendor-kubai <tarball> 按 icons.toml 重取 design/kubai/ 里的上游原件。"""

# 这个脚本不在核心产物的键里：它只改生成的界面代码，不碰 Rust 核心（见 .github/workflows/README.md 的“核心产物”）。

import base64
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
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DESIGN = ROOT / "design"
IOS = ROOT / "apps/ios/Wonton"
COMPOSE = ROOT / "apps/compose/shared/src/androidMain"
KOTLIN = COMPOSE / "kotlin/app/wonton/ui"
WEB_GENERATED = ROOT / "crates/wonton-client/src/pages/bridge/generated"
GENERATED_DIRS = [
    IOS / "Design/Generated",
    IOS / "Assets.xcassets/Icons",
    IOS / "Assets.xcassets/IconsColor",
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
ICON_BACKGROUNDS = ["#FFFFFF", "#F4F4F4", "#0E0E0E", "#262626"]
# WCAG 1.4.11 对图形的最低要求。
ICON_MIN_CONTRAST = 3.0
# 通道差小于它的是白、黑、灰，换色时不动。
KUBAI_GRAY_SPREAD = 6
# 上游的十六进制颜色是四舍五入过的，按主色混白混黑反推时留这点误差（0–255）。
KUBAI_MIX_TOLERANCE = 2
SVG_HEX = re.compile(r'(?<==")#([0-9A-Fa-f]{6}|[0-9A-Fa-f]{3})(?=")')
SVG_MATRIX = re.compile(r'values="([^"]+)"')
SVG_PAINT = re.compile(r"#[0-9A-F]{6}|url\(#[\w-]+\)")
SVG_ELEMENTS = {
    "svg", "defs", "g", "path", "rect", "circle", "ellipse", "text", "linearGradient", "stop", "mask", "clipPath",
    "filter", "feFlood", "feColorMatrix", "feOffset", "feGaussianBlur", "feComposite", "feBlend",
}
SVG_ATTRIBUTES = {
    "xmlns", "viewBox", "width", "height", "id", "x", "y", "x1", "y1", "x2", "y2", "cx", "cy", "r", "rx", "ry", "d",
    "transform", "opacity", "fill", "fill-opacity", "fill-rule", "clip-path", "clip-rule", "mask", "maskUnits",
    "filter", "stroke", "stroke-width", "stroke-opacity", "stroke-linecap", "stroke-linejoin", "stroke-dasharray",
    "stroke-dashoffset", "pathLength", "offset", "stop-color", "gradientUnits", "color-interpolation-filters",
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
    cases = "\n".join(f"    case {name}" for name, _, _ in icon_variants(icons))
    # 实心版没有自己的彩色图：彩色分层不分线框和实心，两个值指向同一张。
    colored = "\n".join(
        f'        .{variant}: "{name}Color",'
        for variant, icon, _ in icon_variants(icons)
        for name in [variant.removesuffix("Fill") if icon.get("fill") else variant]
        if "kubai" in icon
    )
    return (
        BANNER
        + "enum WontonIcon: String {\n" + cases + "\n}\n\n"
        + "extension WontonIcon {\n"
        + "    /// 彩色分层版的资源名；不在表里的只有单色版。\n"
        + "    static let colorAssets: [WontonIcon: String] = [\n" + colored + "\n    ]\n}\n"
    )


def ios_assets(icons):
    files = {
        IOS / "Assets.xcassets/Contents.json": json_bytes(XCASSETS_INFO),
        IOS / "Assets.xcassets/Icons/Contents.json": json_bytes(XCASSETS_INFO),
    }
    for name, icon, style in icon_variants(icons):
        folder = IOS / f"Assets.xcassets/Icons/{name}.imageset"
        image = {"filename": f"{name}.svg", "idiom": "universal"}
        if icon.get("mirror"):
            image["language-direction"] = "left-to-right"
        contents = {
            "images": [image],
            **XCASSETS_INFO,
            "properties": {"preserves-vector-representation": True, "template-rendering-intent": "template"},
        }
        files[folder / "Contents.json"] = json_bytes(contents)
        files[folder / f"{name}.svg"] = upstream_svg(icon, style).read_bytes()
    files.update(ios_color_assets(icons))
    return files


def ios_color_assets(icons):
    """彩色分层版：原色渲染，保留矢量。"""
    files = {IOS / "Assets.xcassets/IconsColor/Contents.json": json_bytes(XCASSETS_INFO)}
    check_palette(icons["palette"])
    sums = kubai_sums()
    used = {f'svg/{icon["kubai"]}.svg' for icon in icons["icons"].values() if "kubai" in icon} | {"LICENSE"}
    if used != set(sums):
        sys.exit(f"design/kubai 里的文件和 icons.toml 用到的对不上：{sorted(used ^ set(sums))}")
    base = hex_rgb(icons["upstream"]["kubai"]["base"])
    for name, icon in icons["icons"].items():
        if "kubai" not in icon:
            continue
        source = kubai_source(f'svg/{icon["kubai"]}.svg', sums)
        svg = kubai_recolored(source.decode(), base, hex_rgb(icons["palette"][icon["color"]]))
        check_plain_svg(name, svg)
        folder = IOS / f"Assets.xcassets/IconsColor/{name}Color.imageset"
        image = {"filename": f"{name}Color.svg", "idiom": "universal"}
        if icon.get("mirror"):
            image["language-direction"] = "left-to-right"
        contents = {
            "images": [image],
            **XCASSETS_INFO,
            "properties": {"preserves-vector-representation": True, "template-rendering-intent": "original"},
        }
        files[folder / "Contents.json"] = json_bytes(contents)
        files[folder / f"{name}Color.svg"] = svg.encode()
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
    for name, value in palette.items():
        for background in ICON_BACKGROUNDS:
            ratio = contrast(hex_rgb(value), hex_rgb(background))
            if ratio < ICON_MIN_CONTRAST:
                sys.exit(f"图标颜色 {name} {value} 在 {background} 上的对比度只有 {ratio:.2f}，要 {ICON_MIN_CONTRAST}")


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


def kubai_recolor(color, base, accent):
    """上游的颜色都是从主色调出来的：多数是主色混白或混黑，深色描边是主色在 OKLab 里压暗并偏一点色相。
    前一种按同样的配比换成新主色；后一种配比对不上，在 OKLCH 里照搬亮度比、彩度比和色相差。"""
    if max(color) - min(color) < KUBAI_GRAY_SPREAD:
        return color
    a, b, error = mix_weights(color, base)
    if error <= KUBAI_MIX_TOLERANCE:
        return tuple(a * x + b * 255 for x in accent)
    (cl, ca, cb), (bl, ba, bb), (al, aa, ab) = oklab(color), oklab(base), oklab(accent)
    chroma = math.hypot(aa, ab) * math.hypot(ca, cb) / math.hypot(ba, bb)
    hue = math.atan2(ab, aa) + math.atan2(cb, ca) - math.atan2(bb, ba)
    return from_oklab((al * cl / bl, chroma * math.cos(hue), chroma * math.sin(hue)))


def kubai_recolored(svg, base, accent):
    def paint(match):
        value = match.group(1)
        value = value if len(value) == 6 else "".join(c * 2 for c in value)
        return rgb_hex(kubai_recolor(hex_rgb("#" + value), base, accent))

    def matrix(match):
        # 投影和高光的颜色写在 feColorMatrix 第 5 列（每行的常数项，0–1）。
        values = match.group(1).split()
        color = kubai_recolor(tuple(float(values[i]) * 255 for i in (4, 9, 14)), base, accent)
        for index, channel in zip((4, 9, 14), color):
            values[index] = f"{max(0, min(255, channel)) / 255:.4f}".rstrip("0").rstrip(".")
        return f'values="{" ".join(values)}"'

    svg = SVG_HEX.sub(paint, svg)
    return SVG_MATRIX.sub(matrix, svg)


def check_plain_svg(name, svg):
    """CoreSVG 不认 CSS：颜色没展开成具体值的（color-mix()、var()、currentColor、style）会画成纯黑。"""
    elements = set(re.findall(r"<([A-Za-z][\w:-]*)", svg))
    attributes = set(re.findall(r'\s([A-Za-z][\w:-]*)="', svg))
    paints = {
        value
        for value in re.findall(r'\s(?:fill|stroke|stop-color|flood-color)="([^"]*)"', svg)
        if value != "none" and not SVG_PAINT.fullmatch(value)
    }
    problems = sorted(elements - SVG_ELEMENTS) + sorted(attributes - SVG_ATTRIBUTES) + sorted(paints)
    if problems:
        sys.exit(f"彩色图标 {name} 里有 CoreSVG 画不了的内容：{problems}")


def kubai_sums():
    lines = (KUBAI / "SHA256SUMS").read_text().splitlines()
    return {path: digest for digest, path in (line.split("  ", 1) for line in lines)}


def kubai_source(path, sums):
    content = (KUBAI / path).read_bytes()
    if hashlib.sha256(content).hexdigest() != sums[path]:
        sys.exit(f"design/kubai/{path} 和 SHA256SUMS 对不上：上游原件不手改")
    return content


def vendor_kubai(tarball):
    """从校验过的 npm tarball 里取出 icons.toml 用到的 SVG 和 LICENSE，重写 design/kubai/。"""
    icons = tomllib.loads((DESIGN / "icons.toml").read_text())
    upstream = icons["upstream"]["kubai"]
    data = Path(tarball).read_bytes()
    algorithm, expected = upstream["integrity"].split("-", 1)
    if base64.b64encode(hashlib.new(algorithm, data).digest()).decode() != expected:
        sys.exit(f"{tarball} 的校验值和 icons.toml 的 integrity 对不上")
    wanted = sorted({f'svg/{icon["kubai"]}.svg' for icon in icons["icons"].values() if "kubai" in icon} | {"LICENSE"})
    if KUBAI.exists():
        shutil.rmtree(KUBAI)
    sums = []
    with tarfile.open(tarball) as archive:
        for path in wanted:
            member = archive.extractfile(f"package/{path}")
            if member is None:
                sys.exit(f"{upstream['package']} {upstream['version']} 里没有 {path}")
            content = member.read()
            (KUBAI / path).parent.mkdir(parents=True, exist_ok=True)
            (KUBAI / path).write_bytes(content)
            sums.append(f"{hashlib.sha256(content).hexdigest()}  {path}")
    (KUBAI / "SHA256SUMS").write_text("\n".join(sums) + "\n")
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
        vendor_kubai(sys.argv[sys.argv.index("--vendor-kubai") + 1])
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
