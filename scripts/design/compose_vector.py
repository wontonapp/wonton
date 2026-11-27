"""把 Kubai 的分层 SVG 变成 Compose 的 ImageVector 源码。

ImageVector 没有滤镜、蒙版和文字，所以先把 SVG 改写成它画得出来的样子（`flatten`），再照着改写后的树出 Kotlin
（`kotlin_file`）。改写后的树仍是合法的 SVG，可以拿去渲染，看 Android 上会是什么样。
"""

import copy
import math
import re
from xml.etree import ElementTree

SVG_NAMESPACE = "http://www.w3.org/2000/svg"
TAG = "{" + SVG_NAMESPACE + "}"
HEX = re.compile(r"#([0-9A-Fa-f]{6}|[0-9A-Fa-f]{3})")
URL = re.compile(r"url\(#([\w-]+)\)")
NUMBER = re.compile(r"[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?")
TRANSFORM = re.compile(r"(translate|rotate|scale)\(([^)]*)\)")
# 上游的画布是 -2.5 -5.5 36 36，四周给投影留了边；图形按 24 的格子画，28 的图才等于 24 的图标。
# ImageVector 画不了投影，直接把视口裁到中间这一块，24dp 的图标就和 iOS 上一样大。
CANVAS = (-2.5, -5.5, 36.0)
VIEWPORT = CANVAS[2] * 24 / 28
ORIGIN = (CANVAS[0] + (CANVAS[2] - VIEWPORT) / 2, CANVAS[1] + (CANVAS[2] - VIEWPORT) / 2)
# 曲线取这么多段来量包围盒：只用来定渐变的起止，差一点看不出来。
SEGMENTS = 16
CAPS = {"round": "Round", "square": "Square", "butt": "Butt"}
JOINS = {"round": "Round", "bevel": "Bevel", "miter": "Miter"}
INHERITED = ("fill", "stroke", "stroke-width", "stroke-linecap", "stroke-linejoin", "fill-opacity", "stroke-opacity",
             "fill-rule", "opacity")
# 上游写的虚线长度是四舍五入到一位小数的，比量出来的路径长度短这么多以内算“一样长”。
DASH_SLACK = 0.1
PARAMETERS = {"M": 2, "L": 2, "H": 1, "V": 1, "C": 6, "S": 4, "Q": 4, "T": 2, "A": 7, "Z": 0}


class Unsupported(Exception):
    """SVG 里有 ImageVector 画不了、这里也没法改写的东西。"""


def number(value):
    return f"{round(float(value), 4):g}"


def path_commands(data):
    """路径拆成 (命令字母, 参数) 的序列；一个字母后面跟几组参数的拆成几条。"""
    commands = []
    for letter, body in re.findall(r"([MmLlHhVvCcSsQqTtAaZz])([^MmLlHhVvCcSsQqTtAaZz]*)", data):
        values = [float(v) for v in NUMBER.findall(body)]
        size = PARAMETERS[letter.upper()]
        if size == 0:
            commands.append((letter, []))
            continue
        if not values or len(values) % size:
            raise Unsupported(f"路径参数个数不对：{letter}{body}")
        for index in range(0, len(values), size):
            # M 后面多出来的坐标按 L 算（SVG 的规定）。
            implicit = {"M": "L", "m": "l"}.get(letter, letter) if index else letter
            commands.append((implicit, values[index:index + size]))
    return commands


def arc_points(start, radii, rotation, large, sweep, end):
    """端点式的椭圆弧上的采样点（SVG 实现附注 F.6.5）。"""
    (x1, y1), (x2, y2), (rx, ry) = start, end, (abs(radii[0]), abs(radii[1]))
    if rx == 0 or ry == 0 or start == end:
        return [end]
    phi = math.radians(rotation)
    cos, sin = math.cos(phi), math.sin(phi)
    dx, dy = (x1 - x2) / 2, (y1 - y2) / 2
    xp, yp = cos * dx + sin * dy, -sin * dx + cos * dy
    scale = math.sqrt(max(1.0, xp * xp / (rx * rx) + yp * yp / (ry * ry)))
    rx, ry = rx * scale, ry * scale
    numerator = rx * rx * ry * ry - rx * rx * yp * yp - ry * ry * xp * xp
    factor = math.sqrt(max(0.0, numerator / (rx * rx * yp * yp + ry * ry * xp * xp)))
    if bool(large) == bool(sweep):
        factor = -factor
    cxp, cyp = factor * rx * yp / ry, -factor * ry * xp / rx
    cx, cy = cos * cxp - sin * cyp + (x1 + x2) / 2, sin * cxp + cos * cyp + (y1 + y2) / 2
    first = math.atan2((yp - cyp) / ry, (xp - cxp) / rx)
    delta = math.atan2((-yp - cyp) / ry, (-xp - cxp) / rx) - first
    if sweep and delta < 0:
        delta += 2 * math.pi
    if not sweep and delta > 0:
        delta -= 2 * math.pi
    points = []
    for step in range(1, SEGMENTS + 1):
        angle = first + delta * step / SEGMENTS
        ex, ey = rx * math.cos(angle), ry * math.sin(angle)
        points.append((cos * ex - sin * ey + cx, sin * ex + cos * ey + cy))
    return points


def bezier_points(controls):
    points = []
    for step in range(1, SEGMENTS + 1):
        t, level = step / SEGMENTS, list(controls)
        while len(level) > 1:
            level = [(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t) for a, b in zip(level, level[1:])]
        points.append(level[0])
    return points


def path_subpaths(data):
    """路径拆成一段一段连续的折线（每个 M 起一段），够用来量包围盒和长度。"""
    subpaths, current, start, last_control = [], (0.0, 0.0), (0.0, 0.0), None
    for letter, values in path_commands(data):
        relative = letter.islower()
        command = letter.upper()
        ox, oy = current if relative else (0.0, 0.0)
        control = None
        if command == "Z":
            if subpaths:
                subpaths[-1].append(start)
            current = start
            continue
        if command == "H":
            segment = [(values[0] + ox, current[1])]
        elif command == "V":
            segment = [(current[0], values[0] + oy)]
        elif command in "ML":
            segment = [(values[0] + ox, values[1] + oy)]
        elif command == "A":
            end = (values[5] + ox, values[6] + oy)
            segment = arc_points(current, values[0:2], values[2], values[3], values[4], end)
        else:
            pairs = [(values[i] + ox, values[i + 1] + oy) for i in range(0, len(values), 2)]
            if command in "ST":
                reflected = current if last_control is None else (
                    2 * current[0] - last_control[0], 2 * current[1] - last_control[1])
                pairs = [reflected] + pairs
            control = pairs[-2]
            segment = bezier_points([current] + pairs)
        if command == "M":
            start = segment[0]
            subpaths.append([])
        elif not subpaths:
            raise Unsupported("路径不是从 M 开始的")
        # 只有同类曲线才接着上一条的控制点对称（S 接 C/S，T 接 Q/T）。
        last_control = control if command in "CSQT" else None
        subpaths[-1].extend(segment)
        current = segment[-1]
    return subpaths


def path_points(data):
    return [point for subpath in path_subpaths(data) for point in subpath]


def path_length(data):
    return sum(math.dist(a, b) for subpath in path_subpaths(data) for a, b in zip(subpath, subpath[1:]))


def shape_path(node):
    """图形的路径数据：圆、椭圆、矩形换成等价的 path。"""
    tag = node.tag.removeprefix(TAG)
    if tag == "path":
        return node.get("d", "")
    get = lambda name: float(node.get(name, "0"))  # noqa: E731
    if tag in ("circle", "ellipse"):
        rx = get("r") if tag == "circle" else get("rx")
        ry = get("r") if tag == "circle" else get("ry")
        cx, cy = get("cx"), get("cy")
        left, right = number(cx - rx), number(cx + rx)
        arc = f"A{number(rx)} {number(ry)} 0 1 0"
        return f"M{left} {number(cy)}{arc} {right} {number(cy)}{arc} {left} {number(cy)}Z"
    if tag == "rect":
        if node.get("rx") or node.get("ry"):
            raise Unsupported("带圆角的 rect")
        return f"M{number(get('x'))} {number(get('y'))}h{number(get('width'))}v{number(get('height'))}h{number(-get('width'))}Z"
    raise Unsupported(f"<{tag}>")


def parents(root):
    return {child: parent for parent in root.iter() for child in parent}


def transform_chain(node, parent_of):
    chain = []
    while node is not None:
        if node.get("transform"):
            chain.append(node.get("transform"))
        node = parent_of.get(node)
    return chain


def flatten(svg):
    """改写成 ImageVector 画得出来的 SVG，返回 (根节点, 文字列表)。

    - 滤镜（投影、高光）去掉：24dp 上看不出差别。
    - 描边蒙版（只留图形外面那半条描边）去掉，改成把描边挪到同一个图形的填充前面画，里面那半条被填充盖住。
    - 角标周围在底图上挖的那一圈缝（另一种蒙版）去掉，角标直接压在底图上。
    - 文字拿出来单独返回，由壳用系统字体画：ImageVector 没有文字，字形也不能从字体里抠出来随 App 分发。
    """
    ElementTree.register_namespace("", SVG_NAMESPACE)
    root = ElementTree.fromstring(svg)
    defs = root.find(TAG + "defs")
    if defs is None:
        defs = ElementTree.SubElement(root, TAG + "defs")
    for node in root.iter():
        node.attrib.pop("filter", None)
    for node in defs.findall(TAG + "filter"):
        defs.remove(node)
    # 上游把裁剪路径和角标的蒙版定义写在图形中间，挪进 defs。
    for parent in list(root.iter()):
        if parent is defs:
            continue
        for child in list(parent):
            if child.tag in (TAG + "mask", TAG + "clipPath"):
                parent.remove(child)
                defs.append(child)
    labels = []
    for parent in list(root.iter()):
        for child in list(parent):
            if child.tag == TAG + "text":
                chain = transform_chain(parent, parents(root))
                if any(Writer.transform(name, values) for item in chain for name, values in TRANSFORM.findall(item)):
                    raise Unsupported("变换里的文字")
                labels.append({name: child.get(name) for name in ("x", "y", "font-size", "font-weight", "fill", "text-anchor")}
                              | {"text": child.text or ""})
                parent.remove(child)
            elif child.get("opacity") == "0":
                parent.remove(child)
    for child in root:
        if child is not defs:
            push_down(child, {})
    move_outlines(root, defs)
    drop_badge_gaps(root, defs)
    for node in defs.findall(TAG + "mask"):
        defs.remove(node)
    return root, labels


def drop_badge_gaps(root, defs):
    """角标的缝：组上挂的蒙版，内容是“整块白，中间一个黑的圆”，在底图上挖出角标周围那一圈。
    去掉它角标就直接压在底图上。别的蒙版不认识，去掉会悄悄画错，所以失败。"""
    masks = {node.get("id"): node for node in defs.findall(TAG + "mask")}
    for node in root.iter():
        match = URL.fullmatch(node.get("mask", ""))
        if "mask" not in node.attrib:
            continue
        shapes = [
            (shape.tag.removeprefix(TAG), shape.get("fill", "").lower())
            for shape in (masks[match.group(1)] if match and match.group(1) in masks else [])
        ]
        if node.tag != TAG + "g" or shapes != [("rect", "#fff"), ("circle", "#000")]:
            raise Unsupported(f'认不出的蒙版 mask="{node.get("mask")}"')
        del node.attrib["mask"]


def push_down(node, inherited):
    """组上写的颜色和描边样式挪到每个图形自己身上：ImageVector 的组只有变换和裁剪。"""
    if node.tag != TAG + "g":
        for name, value in inherited.items():
            if name == "opacity":
                node.set(name, number(float(value) * float(node.get(name, "1"))))
            else:
                node.attrib.setdefault(name, value)
        return
    inherited = dict(inherited)
    for name in INHERITED:
        if name in node.attrib:
            value = node.attrib.pop(name)
            inherited[name] = number(float(value) * float(inherited.get(name, "1"))) if name == "opacity" else value
    for child in node:
        push_down(child, inherited)


def move_outlines(root, defs):
    """描边蒙版的内容是“整块白，中间一个黑的图形”，被它罩住的是同一个图形的描边。"""
    masks = {node.get("id"): node for node in defs.findall(TAG + "mask")}
    parent_of = parents(root)
    drawn = [node for child in root if child is not defs for node in child.iter()]
    for index, node in enumerate(drawn):
        match = URL.fullmatch(node.get("mask", ""))
        if not match or node.tag != TAG + "path" or node.get("fill", "none") != "none":
            continue
        holes = [shape for shape in masks[match.group(1)] if shape.get("fill", "").lower() in ("#000", "#000000")]
        if len(holes) != 1 or shape_path(holes[0]) != node.get("d"):
            raise Unsupported(f"描边蒙版 {match.group(1)} 罩的不是它自己的图形")
        body = next(
            (other for other in reversed(drawn[:index])
             if other.tag == TAG + "path" and other.get("d") == node.get("d") and other.get("fill", "none") != "none"),
            None,
        )
        if body is None:
            raise Unsupported(f"描边蒙版 {match.group(1)} 前面没有同一个图形的填充")
        if transform_chain(body, parent_of) != transform_chain(node, parent_of):
            raise Unsupported(f"描边蒙版 {match.group(1)} 和它的填充不在同一个变换里")
        if any(float(body.get(name, "1")) < 1 for name in ("opacity", "fill-opacity")):
            # 填充半透明就盖不住里面那半条描边，挪到前面画不等价。
            raise Unsupported(f"描边蒙版 {match.group(1)} 的填充是半透明的")
        parent_of[node].remove(node)
        home = parent_of[body]
        home.insert(list(home).index(body), node)
        parent_of[node] = home
        del node.attrib["mask"]


def source_colors(root):
    """图里用到的颜色，按出现的先后；换色表和 Kotlin 里的下标都按这个顺序。"""
    colors = []
    for node in root.iter():
        for attribute in ("fill", "stroke", "stop-color"):
            value = node.get(attribute, "")
            if HEX.fullmatch(value) and long_hex(value) not in colors:
                colors.append(long_hex(value))
    return colors


def long_hex(value):
    value = value if len(value) == 7 else "#" + "".join(c * 2 for c in value[1:])
    return value.upper()


def transform_step(name, values):
    """一步变换的参数（Compose 的 group 参数名）；恒等变换返回空。"""
    values = [float(v) for v in NUMBER.findall(values)]
    if name == "translate":
        x, y = values[0], values[1] if len(values) > 1 else 0.0
        return {"translationX": x, "translationY": y} if x or y else {}
    if name == "rotate":
        if not values[0] % 360:
            return {}
        return {"rotate": values[0]} | ({"pivotX": values[1], "pivotY": values[2]} if len(values) == 3 else {})
    x, y = values[0], values[1] if len(values) > 1 else values[0]
    return {"scaleX": x, "scaleY": y} if (x, y) != (1.0, 1.0) else {}


def silhouette(root):
    """单色平面版：所有颜色换成不透明的黑，只留形状。系统小组件画不了分层图，整张图由系统染成一个颜色。
    白色的字和纸面在平面版里没法挖空，带白色的图不能这样用。"""
    flat = copy.deepcopy(root)
    defs = flat.find(TAG + "defs")
    for node in flat.iter():
        for attribute in ("fill", "stroke"):
            value = node.get(attribute, "none")
            if value == "none":
                continue
            if HEX.fullmatch(value) and long_hex(value) == "#FFFFFF":
                raise Unsupported("带白色的图出不了平面版")
            node.set(attribute, "#000000")
        for attribute in ("opacity", "fill-opacity", "stroke-opacity"):
            node.attrib.pop(attribute, None)
    for node in defs.findall(TAG + "linearGradient"):
        defs.remove(node)
    return flat


def cropped_svg(root):
    """裁掉给投影留的边，图形占满画布。"""
    root = copy.deepcopy(root)
    root.set("viewBox", f"{number(ORIGIN[0])} {number(ORIGIN[1])} {number(VIEWPORT)} {number(VIEWPORT)}")
    return ElementTree.tostring(root, encoding="unicode") + "\n"


VECTOR_DRAWABLE_GROUP = {
    "translationX": "translateX", "translationY": "translateY", "rotate": "rotation", "pivotX": "pivotX",
    "pivotY": "pivotY", "scaleX": "scaleX", "scaleY": "scaleY",
}


def vector_drawable(banner, root):
    """平面版出成 Android 的 VectorDrawable（Glance 的小组件只收资源，画不了 ImageVector）。"""
    lines = [
        '<?xml version="1.0" encoding="utf-8"?>',
        f"<!-- {banner} -->",
        '<vector xmlns:android="http://schemas.android.com/apk/res/android"',
        '    android:width="24dp"',
        '    android:height="24dp"',
        f'    android:viewportWidth="{number(VIEWPORT)}"',
        f'    android:viewportHeight="{number(VIEWPORT)}">',
        f'    <group android:translateX="{number(-ORIGIN[0])}" android:translateY="{number(-ORIGIN[1])}">',
    ]
    defs = root.find(TAG + "defs")

    def walk(node):
        if node.get("clip-path"):
            raise Unsupported("平面版里的裁剪路径")
        steps = [transform_step(name, values) for name, values in TRANSFORM.findall(node.get("transform", ""))]
        if TRANSFORM.sub("", node.get("transform", "")).strip():
            raise Unsupported(f'transform="{node.get("transform")}"')
        steps = [step for step in steps if step]
        for step in steps:
            attributes = " ".join(f'android:{VECTOR_DRAWABLE_GROUP[key]}="{number(value)}"' for key, value in step.items())
            lines.append(f"        <group {attributes}>")
        if node.tag == TAG + "g":
            for child in node:
                walk(child)
        else:
            data = shape_path(node)
            attributes = [f'android:pathData="{data}"']
            if node.get("fill", "none") != "none":
                attributes.append('android:fillColor="#FF000000"')
                if node.get("fill-rule") == "evenodd":
                    attributes.append('android:fillType="evenOdd"')
            if node.get("stroke", "none") != "none":
                check_dash(node, data)
                attributes.append('android:strokeColor="#FF000000"')
                attributes.append(f'android:strokeWidth="{number(node.get("stroke-width", "1"))}"')
                for name, key in (("stroke-linecap", "strokeLineCap"), ("stroke-linejoin", "strokeLineJoin")):
                    if node.get(name):
                        attributes.append(f'android:{key}="{node.get(name)}"')
            if len(attributes) > 1:
                lines.append(f"        <path {' '.join(attributes)} />")
        lines.extend("        </group>" for _ in steps)

    for child in root:
        if child is not defs:
            walk(child)
    lines += ["    </group>", "</vector>", ""]
    return "\n".join(lines)


class Writer:
    """照改写后的树出 `ImageVector.Builder` 的调用。路径数据收成常量，三套颜色共用一份。"""

    def __init__(self, root, colors):
        self.colors = colors
        self.paths = []
        self.lines = []
        defs = root.find(TAG + "defs")
        self.gradients = {node.get("id"): node for node in defs.findall(TAG + "linearGradient")}
        self.clips = {node.get("id"): node for node in defs.findall(TAG + "clipPath")}
        self.group(f"translationX = {number(-ORIGIN[0])}f, translationY = {number(-ORIGIN[1])}f",
                   [child for child in root if child is not defs])

    def constant(self, data):
        if data not in self.paths:
            self.paths.append(data)
        return f"P{self.paths.index(data)}"

    def group(self, arguments, children):
        self.lines.append(f"addGroup({arguments})")
        for child in children:
            self.node(child)
        self.lines.append("clearGroup()")

    def node(self, node):
        wrappers = [self.transform(name, values) for name, values in TRANSFORM.findall(node.get("transform", ""))]
        if TRANSFORM.sub("", node.get("transform", "")).strip():
            raise Unsupported(f'transform="{node.get("transform")}"')
        clip = URL.fullmatch(node.get("clip-path", ""))
        if clip:
            shapes = list(self.clips[clip.group(1)])
            if any(shape.get("transform") or shape.get("clip-rule") for shape in shapes):
                raise Unsupported("带变换或缠绕规则的裁剪路径")
            data = "".join(shape_path(shape) for shape in shapes)
            wrappers.append(f"clipPathData = addPathNodes({self.constant(data)})")
        wrappers = [wrapper for wrapper in wrappers if wrapper]
        for wrapper in wrappers:
            self.lines.append(f"addGroup({wrapper})")
        if node.tag == TAG + "g":
            if any(name in node.attrib for name in ("fill", "stroke", "opacity")):
                raise Unsupported("组上的颜色或不透明度")
            for child in node:
                self.node(child)
        else:
            self.shape(node)
        self.lines.extend("clearGroup()" for _ in wrappers)

    @staticmethod
    def transform(name, values):
        step = transform_step(name, values)
        return ", ".join(f"{key} = {number(value)}f" for key, value in step.items()) if step else None

    def shape(self, node):
        data = shape_path(node)
        opacity = float(node.get("opacity", "1"))
        arguments = [f"pathData = addPathNodes({self.constant(data)})"]
        for attribute, name in (("fill", "fill"), ("stroke", "stroke")):
            brush = self.brush(node.get(attribute, "none"), data)
            if brush is None:
                continue
            arguments.append(f"{name} = {brush}")
            alpha = opacity * float(node.get(f"{attribute}-opacity", "1"))
            if alpha != 1:
                arguments.append(f"{name}Alpha = {number(alpha)}f")
        if len(arguments) == 1:
            return
        if node.get("stroke", "none") != "none":
            check_dash(node, data)
            arguments.append(f'strokeLineWidth = {number(node.get("stroke-width", "1"))}f')
            if node.get("stroke-linecap"):
                arguments.append(f'strokeLineCap = StrokeCap.{CAPS[node.get("stroke-linecap")]}')
            if node.get("stroke-linejoin"):
                arguments.append(f'strokeLineJoin = StrokeJoin.{JOINS[node.get("stroke-linejoin")]}')
        if node.get("fill-rule") == "evenodd":
            arguments.append("pathFillType = PathFillType.EvenOdd")
        self.lines.append(f"addPath({', '.join(arguments)})")

    def brush(self, paint, data):
        if paint == "none":
            return None
        if HEX.fullmatch(paint):
            return f"SolidColor(Color(c[{self.colors.index(long_hex(paint))}]))"
        gradient = self.gradients[URL.fullmatch(paint).group(1)]
        if (gradient.get("x1"), gradient.get("y1"), gradient.get("x2"), gradient.get("y2")) != ("0", "0", "0", "1") \
                or gradient.get("gradientUnits"):
            raise Unsupported("不是从上到下、按图形包围盒算的渐变")
        ys = [y for _, y in path_points(data)]
        stops = ", ".join(
            f'{number(stop.get("offset"))}f to Color(c[{self.colors.index(long_hex(stop.get("stop-color")))}])'
            for stop in gradient
        )
        return f"Brush.verticalGradient({stops}, startY = {number(min(ys))}f, endY = {number(max(ys))}f)"


def check_dash(node, data):
    """上游拿“一段和路径一样长的虚线”做描线动画的起点，静止时就是实线。别的虚线 ImageVector 画不了。"""
    dash = node.get("stroke-dasharray")
    if dash is None:
        return
    values = NUMBER.findall(dash)
    solid = len(values) == 1 and NUMBER.fullmatch(dash.strip()) and float(node.get("stroke-dashoffset", "0")) == 0 \
        and float(values[0]) >= path_length(data) - DASH_SLACK
    if not solid:
        raise Unsupported(f'虚线描边 stroke-dasharray="{dash}"')


def kotlin_string(text):
    return text.replace("\\", "\\\\").replace('"', '\\"').replace("$", "\\$")


def kotlin_label(label, colors):
    """文字相对裁过的视口的位置和大小（0–1），壳按图标实际大小换算。"""
    if label["fill"] is None or not HEX.fullmatch(label["fill"]):
        raise Unsupported("文字的颜色不是具体颜色")
    if label["text-anchor"] != "middle":
        # 壳按“x 是字的中线”摆。
        raise Unsupported(f'文字不是居中对齐的（text-anchor="{label["text-anchor"]}"）')
    return (
        f'WontonGlyphLabel(text = "{kotlin_string(label["text"])}", '
        f'centerX = {number((float(label["x"]) - ORIGIN[0]) / VIEWPORT)}f, '
        f'baseline = {number((float(label["y"]) - ORIGIN[1]) / VIEWPORT)}f, '
        f'size = {number(float(label["font-size"]) / VIEWPORT)}f, '
        f'bold = {"true" if int(label["font-weight"] or 400) >= 600 else "false"}, '
        f'color = {colors.index(long_hex(label["fill"]))})'
    )


def kotlin_file(banner, name, root, labels, palettes, mirror):
    """一个图标一个文件。palettes：light、dark、mono 三套颜色，各是和 source_colors 等长的 0xAARRGGBB 列表。"""
    colors = source_colors(root)
    for label in labels:
        if long_hex(label["fill"]) not in colors:
            colors.append(long_hex(label["fill"]))
    writer = Writer(root, colors)
    constants = "\n".join(f'private const val P{index} = "{data}"' for index, data in enumerate(writer.paths))
    body = "\n".join(f"    {line}" for line in writer.lines)
    tables = "\n".join(
        f'        {variant} = longArrayOf({", ".join(palettes[variant])}),' for variant in ("light", "dark", "mono")
    )
    label = f"        label = {kotlin_label(labels[0], colors)},\n" if labels else ""
    if len(labels) > 1:
        raise Unsupported("不止一处文字")
    used = {
        "Brush": "Brush.", "Color": "Color(", "PathFillType": "PathFillType.", "SolidColor": "SolidColor(",
        "StrokeCap": "StrokeCap.", "StrokeJoin": "StrokeJoin.",
    }
    imports = [f"androidx.compose.ui.graphics.{name}" for name, mark in used.items() if mark in body]
    imports += ["androidx.compose.ui.graphics.vector.ImageVector", "androidx.compose.ui.graphics.vector.addPathNodes",
                "androidx.compose.ui.unit.dp", "app.wonton.ui.design.WontonGlyph"]
    if labels:
        imports.append("app.wonton.ui.design.WontonGlyphLabel")
    imports = "\n".join(f"import {name}" for name in sorted(imports))
    return f'''{banner}@file:Suppress("MagicNumber", "MaxLineLength", "LongMethod", "UnusedReceiverParameter")

package app.wonton.ui.icons

{imports}

{constants}

private fun art(name: String, c: LongArray): ImageVector = ImageVector.Builder(
    name = name,
    defaultWidth = 24.dp,
    defaultHeight = 24.dp,
    viewportWidth = {number(VIEWPORT)}f,
    viewportHeight = {number(VIEWPORT)}f,
    autoMirror = {"true" if mirror else "false"},
).apply {{
{body}
}}.build()

val WontonIcon.{name}: WontonGlyph by lazy(LazyThreadSafetyMode.NONE) {{
    WontonGlyph(
        name = "{name}",
{tables}
{label}        build = ::art,
    )
}}
'''
