#!/usr/bin/env python3
"""从 design/app-icon/mascot/cutout.png 生成两端的 App 图标和启动屏。

形状、角度、神态都不改：主人原图抠出来的角色原样缩放居中。App 图标放在蓝到白的渐变底上，
iOS 出浅色、深色、着色三套，Android 出自适应图标的前景、背景、单色。
启动屏只放角色本身，iOS 出浅色、深色两张，Android 出一张和通知栏的单色剪影。

要 Pillow（pip install pillow）。渲染不依赖 Chrome 版本，CI 不比对，改完把生成结果一起提交。
"""

import json
import sys
from pathlib import Path

try:
    from PIL import Image, ImageDraw, ImageFilter, ImageOps
except ImportError:
    sys.exit("要 Pillow：python3 -m pip install pillow")

ROOT = Path(__file__).resolve().parent.parent
DESIGN = ROOT / "design/app-icon/mascot"
XCASSETS = ROOT / "apps/ios/Wonton/Assets.xcassets"
RES = ROOT / "apps/compose/androidApp/src/main/res"
XML_BANNER = "<!-- 由 scripts/make-app-icon.py 从 design/app-icon/mascot/ 生成，不要手改。 -->\n"
XCASSETS_INFO = {"info": {"author": "xcode", "version": 1}}
DARK = [{"appearance": "luminosity", "value": "dark"}]
TINTED = [{"appearance": "luminosity", "value": "tinted"}]

# Xcode “All Sizes” 下 iOS 的全部槽位（点数，倍数）。Xcode 27.2 实测：只要单独放了一个小尺寸，
# actool 就不再从 1024 缩放出其他尺寸进 Assets.car，所以要放全。
IOS_SIZES = [
    ("20", 2), ("20", 3), ("29", 2), ("29", 3), ("38", 2), ("38", 3), ("40", 2), ("40", 3),
    ("60", 2), ("60", 3), ("64", 2), ("64", 3), ("68", 2), ("76", 2), ("83.5", 2),
]

# 上蓝下白。蓝比角色身体（#C8DBE9）深一档，白角和头顶在蓝上分得开。深色版是夜里的一团深蓝。
TOP = (126, 186, 240)
BOTTOM = (255, 255, 255)
DARK_TOP = (30, 48, 82)
DARK_BOTTOM = (12, 20, 38)
# 角色最长边占画布的比例（主人定的版式：角色约占 75–80%）。
FILL = 0.80
# Android 安全区是 108dp 里中间 66dp 的圆；角色最长边留在 0.52 以内，露出的 72dp 里约占 78%，和 iOS 一样大。
ANDROID_FILL = 0.52
# Android 自适应图标画布 108dp，这里按 4 倍出图。
ANDROID_SIDE = 432

# 启动屏的底色：米白和墨色。App 图标铺蓝白渐变，启动屏只放角色本身，不铺方底。
LAUNCH_PAPER = (243, 237, 226)
LAUNCH_INK = (30, 27, 24)
# 启动屏画布在 iOS 上的边长（pt），角色占其中 0.8。
LAUNCH_POINTS = 160
# Android 12 把启动图标画在 288dp 的画布上，只露出中间 192dp 的圆：108dp 视口里角色最长边
# 留在 192/288*108 ≈ 0.6 以内，圆边切不到绒毛。
ANDROID_LAUNCH_FILL = 0.60
# 启动屏按 288dp 画布的 3 倍出图，3 倍屏上不放大。
ANDROID_LAUNCH_SIDE = 864
# 单色版里视为“五官”的亮度上限，和挖空处的最小面积（像素）。
FACE_DARK = 110
FACE_MIN_AREA = 150


def gradient(size, top, bottom):
    column = Image.new("RGB", (1, 256))
    for y in range(256):
        t = y / 255
        column.putpixel((0, y), tuple(round(a + (b - a) * t) for a, b in zip(top, bottom)))
    return column.resize((size, size), Image.BICUBIC)


def character():
    source = Image.open(DESIGN / "cutout.png")
    if source.mode != "RGBA" or source.getchannel("A").getextrema()[0] == 255:
        sys.exit("design/app-icon/mascot/cutout.png 要是带透明边的 RGBA 抠图，没有 alpha 会把整张画布带进来")
    image = source.convert("RGBA")
    return image.crop(image.split()[-1].getbbox())


def place(canvas_size, fill, body, shadow=True):
    """角色按最长边缩放、视觉居中（略下移，角在上方占了空），返回透明层。"""
    scale = fill * canvas_size / max(body.size)
    scaled = body.resize((round(body.width * scale), round(body.height * scale)), Image.LANCZOS)
    layer = Image.new("RGBA", (canvas_size, canvas_size), (0, 0, 0, 0))
    x = (canvas_size - scaled.width) // 2
    y = (canvas_size - scaled.height) // 2 + round(canvas_size * 0.02)
    if shadow:
        # 正下方一团柔和的接触阴影，让角色落在底上
        shade = Image.new("RGBA", layer.size, (0, 0, 0, 0))
        width = scaled.width * 0.55
        cx, cy = canvas_size / 2, y + scaled.height * 0.97
        rect = (cx - width / 2, cy - width * 0.07, cx + width / 2, cy + width * 0.07)
        ImageDraw.Draw(shade).ellipse(rect, fill=(40, 70, 110, 70))
        layer = Image.alpha_composite(layer, shade.filter(ImageFilter.GaussianBlur(canvas_size * 0.02)))
    layer.alpha_composite(scaled, (x, y))
    return layer


def ios_light(body):
    return Image.alpha_composite(gradient(1024, TOP, BOTTOM).convert("RGBA"), place(1024, FILL, body)).convert("RGB")


def ios_dark(body):
    return Image.alpha_composite(
        gradient(1024, DARK_TOP, DARK_BOTTOM).convert("RGBA"), place(1024, FILL, body, shadow=False)
    ).convert("RGB")


def ios_tinted(body):
    # 着色版交灰阶图，系统按用户选的颜色上色；底是黑的
    layer = place(1024, FILL, body, shadow=False)
    gray = ImageOps.grayscale(layer.convert("RGB"))
    out = Image.new("L", (1024, 1024), 0)
    out.paste(gray, (0, 0), layer.split()[-1])
    return out.convert("RGB")


def android_foreground(body):
    return place(ANDROID_SIDE, ANDROID_FILL, body)


def keep_blobs(mask, min_area):
    from collections import deque

    width, height = mask.size
    pixels = mask.load()
    seen = bytearray(width * height)
    out = Image.new("L", (width, height), 0)
    out_pixels = out.load()
    for y0 in range(height):
        for x0 in range(width):
            if seen[y0 * width + x0] or not pixels[x0, y0]:
                continue
            queue = deque([(x0, y0)])
            seen[y0 * width + x0] = 1
            cells = []
            while queue:
                x, y = queue.popleft()
                cells.append((x, y))
                for nx, ny in ((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)):
                    if 0 <= nx < width and 0 <= ny < height and not seen[ny * width + nx] and pixels[nx, ny]:
                        seen[ny * width + nx] = 1
                        queue.append((nx, ny))
            if len(cells) >= min_area:
                for x, y in cells:
                    out_pixels[x, y] = 255
    return out


def android_monochrome(body, fill=ANDROID_FILL):
    # 单色版只用透明度：身体是白色剪影，眼睛和嘴挖空，系统按主题上色。
    # 剪影取抠图的 alpha，不按亮度切：毛边的深浅是绒毛本身，拿来切边一圈毛刺。
    layer = place(ANDROID_SIDE, fill, body, shadow=False)
    mask = layer.split()[-1].point(lambda v: 255 if v > 128 else 0)
    mask = mask.filter(ImageFilter.MaxFilter(5)).filter(ImageFilter.MinFilter(5))
    # 往里收 4px 再找五官，边缘的绒毛不参与，碎点按面积扔掉
    inner = mask.filter(ImageFilter.MinFilter(9))
    dark = ImageOps.grayscale(layer.convert("RGB")).point(lambda v: 255 if v < FACE_DARK else 0)
    feats = keep_blobs(Image.composite(dark, Image.new("L", mask.size, 0), inner), FACE_MIN_AREA)
    # 只在脸这一条带上挖：角的条纹在角根横切，挖掉会把角切断
    x0, y0, x1, y1 = mask.getbbox()
    band = Image.new("L", mask.size, 0)
    ImageDraw.Draw(band).rectangle(
        (x0, y0 + int((y1 - y0) * 0.26), x1, y0 + int((y1 - y0) * 0.82)), fill=255
    )
    feats = Image.composite(feats, Image.new("L", mask.size, 0), band)
    out = Image.new("RGBA", mask.size, (255, 255, 255, 0))
    out.putalpha(Image.composite(Image.new("L", mask.size, 0), mask, feats))
    return out


def write_json(path, content):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(content, indent=2) + "\n")


def replace_dir(path):
    import shutil

    if path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True)


def write_text(path, content):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)


def ios_app_icon(body):
    directory = XCASSETS / "AppIcon.appiconset"
    replace_dir(directory)
    variants = [("light", None, ios_light), ("dark", DARK, ios_dark), ("tinted", TINTED, ios_tinted)]
    images = []
    for name, appearances, draw in variants:
        extra = {"appearances": appearances} if appearances else {}
        full = draw(body)
        by_pixels = {}
        for points, scale in IOS_SIZES:
            side = round(float(points) * scale)
            # 同像素的尺寸（40@3x 和 60@2x 都是 120px）只存一份文件，两条都指过去。
            filename = by_pixels.get(side) or f"{name}-{points}@{scale}x.png"
            if side not in by_pixels:
                full.resize((side, side), Image.LANCZOS).save(directory / filename, optimize=True)
                by_pixels[side] = filename
            images.append(
                {
                    **extra,
                    "filename": filename,
                    "idiom": "universal",
                    "platform": "ios",
                    "scale": f"{scale}x",
                    "size": f"{points}x{points}",
                }
            )
        # 1024 只给商店校验用，默认外观一份就够；dark/tinted 的 1024 可选，不放省约 1 MB。
        if appearances is None:
            filename = f"{name}-1024.png"
            full.save(directory / filename, optimize=True)
            images.append({**extra, "filename": filename, "idiom": "universal", "platform": "ios", "size": "1024x1024"})
    write_json(directory / "Contents.json", {"images": images, **XCASSETS_INFO})


def ios_launch(body):
    background = XCASSETS / "LaunchBackground.colorset"
    replace_dir(background)
    write_json(
        background / "Contents.json",
        {
            "colors": [
                {"color": color_components(hex_color(LAUNCH_PAPER)), "idiom": "universal"},
                {"appearances": DARK, "color": color_components(hex_color(LAUNCH_INK)), "idiom": "universal"},
            ],
            **XCASSETS_INFO,
        },
    )
    glyph = XCASSETS / "LaunchGlyph.imageset"
    replace_dir(glyph)
    images = []
    for name, appearances in [("light", None), ("dark", DARK)]:
        extra = {"appearances": appearances} if appearances else {}
        for scale in (2, 3):
            side = LAUNCH_POINTS * scale
            filename = f"{name}@{scale}x.png"
            place(side, FILL, body, shadow=False).save(glyph / filename, optimize=True)
            images.append({**extra, "filename": filename, "idiom": "universal", "scale": f"{scale}x"})
    write_json(glyph / "Contents.json", {"images": images, **XCASSETS_INFO})


def android(body):
    write_text(
        RES / "values/app_icon_gradient.xml",
        android_colors(
            [("app_icon_gradient_top", hex_color(TOP)), ("app_icon_gradient_bottom", hex_color(BOTTOM))]
        ),
    )
    write_text(RES / "drawable/ic_launcher_background.xml", android_background_shape())
    obsolete = RES / "drawable/ic_launcher_foreground.xml"
    if obsolete.exists():
        obsolete.unlink()
    (RES / "drawable-nodpi").mkdir(parents=True, exist_ok=True)
    android_foreground(body).save(RES / "drawable-nodpi/ic_launcher_foreground.png")
    monochrome = android_monochrome(body)
    monochrome.save(RES / "drawable-nodpi/ic_launcher_monochrome.png")
    # 通知栏小图标单独出：自适应图标 0.52 的安全区在 24dp 的通知图标里太小。
    android_monochrome(body, FILL).save(RES / "drawable-nodpi/notification_glyph.png", optimize=True)
    write_text(RES / "mipmap-anydpi/ic_launcher.xml", android_adaptive_icon())
    android_launch(body)


def android_colors(entries):
    lines = "".join(f'    <color name="{name}">{value.upper()}</color>\n' for name, value in entries)
    return f'<?xml version="1.0" encoding="utf-8"?>\n{XML_BANNER}<resources>\n{lines}</resources>\n'


def hex_color(rgb):
    return "#%02X%02X%02X" % rgb


def color_components(value):
    red, green, blue = value[1:3], value[3:5], value[5:7]
    return {
        "color-space": "srgb",
        "components": {"alpha": "1.000", "blue": f"0x{blue.upper()}", "green": f"0x{green.upper()}", "red": f"0x{red.upper()}"},
    }


def android_bool(value):
    return (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        + XML_BANNER
        + "<resources>\n"
        + f'    <bool name="launch_light_system_bars">{value}</bool>\n</resources>\n'
    )


def android_launch(body):
    # 启动屏的底色随深色模式翻转；图标不翻，桌面上的图标不该跟着系统外观变。
    write_text(
        RES / "values/app_icon_colors.xml",
        android_colors([("launch_background", hex_color(LAUNCH_PAPER))]),
    )
    write_text(
        RES / "values-night/app_icon_colors.xml",
        android_colors([("launch_background", hex_color(LAUNCH_INK))]),
    )
    write_text(RES / "values/app_icon_bools.xml", android_bool("true"))
    write_text(RES / "values-night/app_icon_bools.xml", android_bool("false"))
    obsolete = RES / "drawable/launch_glyph.xml"
    if obsolete.exists():
        obsolete.unlink()
    (RES / "drawable-nodpi").mkdir(parents=True, exist_ok=True)
    place(ANDROID_LAUNCH_SIDE, ANDROID_LAUNCH_FILL, body, shadow=False).save(
        RES / "drawable-nodpi/launch_glyph.png", optimize=True
    )


def android_background_shape():
    # angle=270 从顶到底；系统按图标遮罩裁切，形状铺满 108dp
    return (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        + XML_BANNER
        + '<shape xmlns:android="http://schemas.android.com/apk/res/android">\n'
        '    <gradient android:angle="270" android:type="linear"\n'
        '        android:startColor="@color/app_icon_gradient_top"\n'
        '        android:endColor="@color/app_icon_gradient_bottom" />\n'
        "</shape>\n"
    )


def android_adaptive_icon():
    return (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        + XML_BANNER
        + '<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n'
        '    <background android:drawable="@drawable/ic_launcher_background" />\n'
        '    <foreground android:drawable="@drawable/ic_launcher_foreground" />\n'
        '    <monochrome android:drawable="@drawable/ic_launcher_monochrome" />\n'
        "</adaptive-icon>\n"
    )


def main():
    body = character()
    ios_app_icon(body)
    ios_launch(body)
    android(body)
    print("App 图标和启动屏已生成")


if __name__ == "__main__":
    main()
