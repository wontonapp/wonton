#!/usr/bin/env python3
"""从 design/app-icon/ 生成启动屏资源。用 macOS 上的 Chrome 无头模式渲染 SVG。App 图标由
scripts/make-app-icon.py 从 design/app-icon/mascot/ 出，两边各管各的输出。"""

import json
import re
import shutil
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DESIGN = ROOT / "design/app-icon"
XCASSETS = ROOT / "apps/ios/Wonton/Assets.xcassets"
RES = ROOT / "apps/compose/androidApp/src/main/res"
CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"

SVG_NS = "{http://www.w3.org/2000/svg}"
XML_BANNER = "<!-- 由 scripts/generate-app-icon.py 从 design/app-icon/ 生成，不要手改。 -->\n"
XCASSETS_INFO = {"info": {"author": "xcode", "version": 1}}
DARK = [{"appearance": "luminosity", "value": "dark"}]

# 启动屏字形在 iOS 上的边长（pt）。
LAUNCH_POINTS = 120
# 源 SVG 单位到 108 视口的缩放。启动屏的画布按 288dp 显示，0.17 对应字高约 120dp，和 iOS 接近。
ANDROID_LAUNCH_SCALE = 0.17


@dataclass(frozen=True)
class Master:
    """一份母版 SVG 里的字形和摆放。"""

    path: str
    center_x: float
    center_y: float
    stroke_width: float
    paper: str
    ink: str

    def glyph_svg(self, side, foreground):
        """紧贴字形的透明底 SVG：字形撑满画布，留一个描边的余量。"""
        span = 262 + self.stroke_width
        return (
            f'<svg xmlns="http://www.w3.org/2000/svg" width="{side}" height="{side}" viewBox="0 0 {span} {span}">'
            f'<g transform="{self._placement(span / 2, 1)}">{self._glyph(foreground)}</g></svg>'
        )

    def _placement(self, origin, scale):
        return f"translate({origin} {origin}) scale({scale}) translate({-self.center_x} {-self.center_y})"

    def _glyph(self, color):
        return (
            f'<path fill="{color}" stroke="{color}" stroke-width="{self.stroke_width}" '
            f'stroke-linejoin="round" d="{self.path}"/>'
        )


def load_master(name):
    root = ET.parse(DESIGN / name).getroot()
    background = root.find(f"{SVG_NS}rect")
    placement = root.find(f"{SVG_NS}g")
    glyph = placement.find(f"{SVG_NS}path")
    matched = re.fullmatch(
        r"translate\(512 512\) scale\(([\d.]+)\) translate\(-([\d.]+) -([\d.]+)\)",
        placement.get("transform"),
    )
    if matched is None:
        sys.exit(f"{name} 的摆放写法变了：{placement.get('transform')}")
    upstream = ET.parse(DESIGN / "yun-oracle.svg").getroot().find(f"{SVG_NS}path").get("d")
    if glyph.get("d") != upstream:
        sys.exit(f"{name} 的字形和 yun-oracle.svg 不一致")
    return Master(
        path=upstream,
        center_x=float(matched[2]),
        center_y=float(matched[3]),
        stroke_width=float(glyph.get("stroke-width")),
        paper=background.get("fill"),
        ink=glyph.get("fill"),
    )


def render(svg, side, out, transparent=False):
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        source = Path(tmp) / "icon.svg"
        source.write_text(svg)
        command = [
            CHROME,
            "--headless=new",
            "--disable-gpu",
            "--hide-scrollbars",
            "--no-first-run",
            f"--user-data-dir={tmp}/profile",
            "--force-device-scale-factor=1",
            f"--window-size={side},{side}",
            f"--screenshot={out}",
        ]
        if transparent:
            command.append("--default-background-color=00000000")
        subprocess.run([*command, source.as_uri()], check=True, capture_output=True)


def write_json(path, content):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(content, indent=2) + "\n")


def replace_dir(path):
    if path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True)


def color_components(value):
    red, green, blue = value[1:3], value[3:5], value[5:7]
    return {
        "color-space": "srgb",
        "components": {"alpha": "1.000", "blue": f"0x{blue.upper()}", "green": f"0x{green.upper()}", "red": f"0x{red.upper()}"},
    }


def ios_launch(main):
    background = XCASSETS / "LaunchBackground.colorset"
    replace_dir(background)
    write_json(
        background / "Contents.json",
        {
            "colors": [
                {"color": color_components(main.paper), "idiom": "universal"},
                {"appearances": DARK, "color": color_components(main.ink), "idiom": "universal"},
            ],
            **XCASSETS_INFO,
        },
    )
    glyph = XCASSETS / "LaunchGlyph.imageset"
    replace_dir(glyph)
    images = []
    for name, appearances, color in [("light", None, main.ink), ("dark", DARK, main.paper)]:
        extra = {"appearances": appearances} if appearances else {}
        for scale in (2, 3):
            side = LAUNCH_POINTS * scale
            filename = f"{name}@{scale}x.png"
            render(main.glyph_svg(side, color), side, glyph / filename, transparent=True)
            images.append({**extra, "filename": filename, "idiom": "universal", "scale": f"{scale}x"})
    write_json(glyph / "Contents.json", {"images": images, **XCASSETS_INFO})


def android_glyph(master, scale, color):
    translate_x = round(54 - master.center_x * scale, 4)
    translate_y = round(54 - master.center_y * scale, 4)
    return (
        XML_BANNER + '<vector xmlns:android="http://schemas.android.com/apk/res/android"\n'
        '    android:width="108dp"\n'
        '    android:height="108dp"\n'
        '    android:viewportWidth="108"\n'
        '    android:viewportHeight="108">\n'
        f'    <group android:scaleX="{scale}" android:scaleY="{scale}"\n'
        f'        android:translateX="{translate_x}" android:translateY="{translate_y}">\n'
        f'        <path android:fillColor="{color}"\n'
        f'            android:strokeColor="{color}"\n'
        f'            android:strokeWidth="{master.stroke_width:g}"\n'
        '            android:strokeLineJoin="round"\n'
        f'            android:pathData="{master.path}" />\n'
        "    </group>\n"
        "</vector>\n"
    )


def android_colors(entries):
    lines = "".join(f'    <color name="{name}">{value.upper()}</color>\n' for name, value in entries)
    return f'<?xml version="1.0" encoding="utf-8"?>\n{XML_BANNER}<resources>\n{lines}</resources>\n'


def android_bool(value):
    return (
        f'<?xml version="1.0" encoding="utf-8"?>\n{XML_BANNER}<resources>\n'
        f'    <bool name="launch_light_system_bars">{value}</bool>\n</resources>\n'
    )


def write_text(path, content):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)


def android(main):
    # 启动屏的颜色随深色模式翻转；App 图标不翻，桌面上的图标不该跟着系统外观变。
    write_text(
        RES / "values/app_icon_colors.xml",
        android_colors([("launch_background", main.paper), ("launch_glyph", main.ink)]),
    )
    write_text(
        RES / "values-night/app_icon_colors.xml",
        android_colors([("launch_background", main.ink), ("launch_glyph", main.paper)]),
    )
    write_text(RES / "values/app_icon_bools.xml", android_bool("true"))
    write_text(RES / "values-night/app_icon_bools.xml", android_bool("false"))
    write_text(
        RES / "drawable/launch_glyph.xml",
        android_glyph(main, ANDROID_LAUNCH_SCALE, "@color/launch_glyph"),
    )


def main():
    if not Path(CHROME).exists():
        sys.exit(f"找不到 Chrome：{CHROME}")
    master = load_master("icon.svg")
    ios_launch(master)
    android(master)
    print("启动屏资源已生成")


if __name__ == "__main__":
    main()
