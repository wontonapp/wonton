#!/usr/bin/env python3
"""从 design/ 生成原生令牌、图标、噪点和客户端网页令牌映射。--check 只比对，不一致时失败。"""

import hashlib
import json
import os
import shutil
import subprocess
import sys
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
    return BANNER + "enum WontonIcon: String {\n" + cases + "\n}\n"


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
    return files


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
