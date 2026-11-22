#!/usr/bin/env python3
"""从 design/app-icon/mascot/cutout.png 生成两端的 App 图标。

形状、角度、神态都不改：主人原图抠出来的角色原样缩放居中，放在蓝到白的渐变底上。
iOS 出浅色、深色、着色三套，Android 出自适应图标的前景、背景、单色。
启动屏的字形仍由 generate-app-icon.py 从 SVG 出，两边互不覆盖。

要 Pillow（pip install pillow）。渲染不依赖 Chrome 版本，CI 不比对，改完把生成结果一起提交。
"""

import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageOps

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
# 单色版里视为“五官”的亮度上限，和挖空处的最小面积（像素）。
FACE_DARK = 110
FACE_MIN_AREA = 40


def gradient(size, top, bottom):
    column = Image.new("RGB", (1, 256))
    for y in range(256):
        t = y / 255
        column.putpixel((0, y), tuple(round(a + (b - a) * t) for a, b in zip(top, bottom)))
    return column.resize((size, size), Image.BICUBIC)


def character():
    image = Image.open(DESIGN / "cutout.png").convert("RGBA")
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


def android_monochrome(body):
    # 单色版只用透明度：身体是白色剪影，眼睛和嘴挖空，系统按主题上色。
    # 剪影取抠图的 alpha，不按亮度切：毛边的深浅是绒毛本身，拿来切边一圈毛刺。
    layer = place(ANDROID_SIDE, ANDROID_FILL, body, shadow=False)
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


def ios_app_icon():
    body = character()
    directory = XCASSETS / "AppIcon.appiconset"
    replace_dir(directory)
    variants = [("light", None, ios_light), ("dark", DARK, ios_dark), ("tinted", TINTED, ios_tinted)]
    images = []
    for name, appearances, draw in variants:
        extra = {"appearances": appearances} if appearances else {}
        full = draw(body)
        for points, scale in IOS_SIZES:
            side = round(float(points) * scale)
            filename = f"{name}-{points}@{scale}x.png"
            full.resize((side, side), Image.LANCZOS).save(directory / filename)
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
        filename = f"{name}-1024.png"
        full.save(directory / filename)
        images.append({**extra, "filename": filename, "idiom": "universal", "platform": "ios", "size": "1024x1024"})
    write_json(directory / "Contents.json", {"images": images, **XCASSETS_INFO})


def android():
    body = character()
    write_text(RES / "values/app_icon_gradient.xml", android_colors())
    write_text(RES / "drawable/ic_launcher_background.xml", android_background_shape())
    obsolete = RES / "drawable/ic_launcher_foreground.xml"
    if obsolete.exists():
        obsolete.unlink()
    (RES / "drawable-nodpi").mkdir(parents=True, exist_ok=True)
    android_foreground(body).save(RES / "drawable-nodpi/ic_launcher_foreground.png")
    android_monochrome(body).save(RES / "drawable-nodpi/ic_launcher_monochrome.png")
    write_text(RES / "mipmap-anydpi/ic_launcher.xml", android_adaptive_icon())


def android_colors():
    return (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        + XML_BANNER
        + "<resources>\n"
        + f'    <color name="app_icon_gradient_top">{hex_color(TOP)}</color>\n'
        + f'    <color name="app_icon_gradient_bottom">{hex_color(BOTTOM)}</color>\n'
        + "</resources>\n"
    )


def hex_color(rgb):
    return "#%02X%02X%02X" % rgb


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
    try:
        import PIL  # noqa: F401
    except ImportError:
        sys.exit("要 Pillow：python3 -m pip install pillow")
    ios_app_icon()
    android()
    print("App 图标已生成")


if __name__ == "__main__":
    main()
