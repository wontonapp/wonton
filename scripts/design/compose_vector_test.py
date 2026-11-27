"""compose_vector 的用例：ImageVector 画不了的东西要么改写成等价的画法，要么失败，不悄悄画错。"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from design import compose_vector as vector  # noqa: E402

SVG = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="-2.5 -5.5 36 36" fill="none">{}</svg>'
CIRCLE = "M4 12.5A11.5 11.5 0 1 0 27 12.5A11.5 11.5 0 1 0 4 12.5Z"
OUTLINE_MASK = (
    '<mask id="ol"><rect x="-40" y="-40" width="111" height="109" fill="#fff"/>'
    f'<path d="{CIRCLE}" fill="#000"/></mask>'
)


def kotlin(body: str, mirror: bool = False) -> str:
    root, labels = vector.flatten(SVG.format(body))
    colors = vector.source_colors(root) + [vector.long_hex(label["fill"]) for label in labels]
    table = [f"0xFF{color[1:]}" for color in colors]
    return vector.kotlin_file("", "X", root, labels, {"light": table, "dark": table, "mono": table}, mirror)


class PathPointsTest(unittest.TestCase):
    def extent(self, data: str) -> tuple[float, float, float, float]:
        points = vector.path_points(data)
        xs, ys = [x for x, _ in points], [y for _, y in points]
        return min(xs), min(ys), max(xs), max(ys)

    def assert_extent(self, data: str, expected: tuple[float, float, float, float]) -> None:
        for got, want in zip(self.extent(data), expected):
            self.assertAlmostEqual(got, want, delta=0.05)

    def test_circle_made_of_two_arcs_reaches_top_and_bottom(self):
        self.assert_extent(CIRCLE, (4, 1, 27, 24))

    def test_relative_lines_and_closing(self):
        self.assert_extent("M2 3h10v5h-10z", (2, 3, 12, 8))

    def test_quadratic_curve_stays_inside_its_control_point(self):
        left, top, right, bottom = self.extent("M0 0Q5 10 10 0")
        self.assertAlmostEqual(bottom, 5, delta=0.05)
        self.assertEqual((left, top, right), (0, 0, 10))

    def test_smooth_cubic_reflects_previous_control(self):
        _, top, _, bottom = self.extent("M0 0C0 10 10 10 10 0S20 -10 20 0")
        self.assertAlmostEqual(bottom, 7.5, delta=0.05)
        self.assertAlmostEqual(top, -7.5, delta=0.05)

    def test_wrong_parameter_count_fails(self):
        with self.assertRaises(vector.Unsupported):
            vector.path_points("M0 0L1")


class FlattenTest(unittest.TestCase):
    def test_outline_mask_becomes_stroke_drawn_before_its_fill(self):
        out = kotlin(
            f"<defs>{OUTLINE_MASK}</defs>"
            f'<path d="{CIRCLE}" fill="#93BBFF"/>'
            f'<path d="{CIRCLE}" mask="url(#ol)" stroke="#5393FF" stroke-width="2"/>'
        )
        self.assertLess(out.index("stroke = SolidColor"), out.index("fill = SolidColor"))

    def test_outline_goes_before_the_nearest_fill_of_the_same_shape(self):
        sheet = (
            '<g transform="translate({} 0)">'
            f'<path d="{CIRCLE}" fill="#93BBFF"/>'
            f'<path d="{CIRCLE}" mask="url(#ol)" stroke="#5393FF" stroke-width="2"/></g>'
        )
        lines = [line.strip() for line in kotlin(f"<defs>{OUTLINE_MASK}</defs>{sheet.format(1)}{sheet.format(9)}").splitlines()]
        second = lines.index("addGroup(translationX = 9f, translationY = 0f)")
        self.assertIn("stroke = SolidColor", lines[second + 1])
        self.assertIn("fill = SolidColor", lines[second + 2])

    def test_outline_mask_around_a_different_shape_fails(self):
        with self.assertRaises(vector.Unsupported):
            kotlin(
                f"<defs>{OUTLINE_MASK}</defs>"
                '<path d="M0 0h4v4z" fill="#93BBFF"/><path d="M0 0h4v4z" mask="url(#ol)" stroke="#5393FF"/>'
            )

    def test_outline_without_a_fill_before_it_fails(self):
        with self.assertRaises(vector.Unsupported):
            kotlin(f'<defs>{OUTLINE_MASK}</defs><path d="{CIRCLE}" mask="url(#ol)" stroke="#5393FF"/>')

    def test_filters_and_invisible_shapes_are_dropped(self):
        out = kotlin(
            '<defs><filter id="sh"><feOffset dy="2"/></filter></defs>'
            f'<g filter="url(#sh)"><path d="{CIRCLE}" fill="#93BBFF"/></g>'
            '<path d="M0 0h1" stroke="#5393FF" opacity="0"/>'
        )
        self.assertEqual(out.count("addPath("), 1)

    def test_group_paint_is_pushed_onto_its_shapes(self):
        out = kotlin('<g stroke="#5393FF" stroke-width="1.6"><path d="M0 0h1"/><path d="M0 2h1" stroke="#FFFFFF"/></g>')
        self.assertEqual(out.count("strokeLineWidth = 1.6f"), 2)
        self.assertIn("Color(c[0])", out)
        self.assertIn("Color(c[1])", out)

    def test_text_is_handed_to_the_shell_as_a_label(self):
        out = kotlin(
            f'<path d="{CIRCLE}" fill="#93BBFF"/>'
            '<text x="15.5" y="18.4" font-size="16" font-weight="700" fill="#FFFFFF">?</text>'
        )
        self.assertIn('WontonGlyphLabel(text = "?", centerX = 0.5f', out)
        self.assertIn("bold = true, color = 1)", out)

    def test_flattened_tree_is_still_svg_without_masks_filters_or_text(self):
        root, _ = vector.flatten(SVG.format(
            f'<defs>{OUTLINE_MASK}<filter id="sh"><feOffset dy="2"/></filter></defs>'
            f'<g filter="url(#sh)"><path d="{CIRCLE}" fill="#93BBFF"/></g>'
            f'<path d="{CIRCLE}" mask="url(#ol)" stroke="#5393FF"/><text x="1" y="1" fill="#FFFFFF">?</text>'
        ))
        tags = {node.tag.removeprefix(vector.TAG) for node in root.iter()}
        self.assertEqual(tags, {"svg", "defs", "g", "path"})
        self.assertFalse(any("mask" in node.attrib or "filter" in node.attrib for node in root.iter()))


class KotlinTest(unittest.TestCase):
    def test_gradient_runs_over_the_shape_it_fills(self):
        out = kotlin(
            '<defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1">'
            '<stop offset="0" stop-color="#93BBFF"/><stop offset="1" stop-color="#7FAEFF"/></linearGradient></defs>'
            f'<path d="{CIRCLE}" fill="url(#g)"/>'
        )
        self.assertIn("Brush.verticalGradient(0f to Color(c[0]), 1f to Color(c[1]), startY = 1f, endY = 24f)", out)

    def test_gradient_in_another_direction_fails(self):
        with self.assertRaises(vector.Unsupported):
            kotlin(
                '<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="0">'
                '<stop offset="0" stop-color="#93BBFF"/></linearGradient></defs>'
                f'<path d="{CIRCLE}" fill="url(#g)"/>'
            )

    def test_transforms_and_clip_become_groups_and_identity_is_skipped(self):
        out = kotlin(
            '<defs><clipPath id="c"><path d="M0 0h9v9z"/></clipPath></defs>'
            '<g transform="translate(3 4) rotate(45) scale(1)" clip-path="url(#c)"><path d="M0 0h1" stroke="#5393FF"/></g>'
        )
        self.assertIn("addGroup(translationX = 3f, translationY = 4f)", out)
        self.assertIn("addGroup(rotate = 45f)", out)
        self.assertNotIn("scaleX", out)
        self.assertIn("addGroup(clipPathData = addPathNodes(P0))", out)
        self.assertEqual(out.count("addGroup("), out.count("clearGroup()"))

    def test_circle_is_drawn_as_a_path_and_same_shape_is_stored_once(self):
        out = kotlin(f'<circle cx="15.5" cy="12.5" r="11.5" fill="#93BBFF"/><path d="{CIRCLE}" stroke="#5393FF"/>')
        self.assertEqual(out.count("private const val P"), 1)
        self.assertEqual(out.count("addPathNodes(P0)"), 2)
        self.assertIn('"M4 12.5A11.5 11.5 0 1 0 27 12.5A11.5 11.5 0 1 0 4 12.5Z"', out)

    def test_opacity_and_stroke_style_carry_over(self):
        out = kotlin(
            '<path d="M0 0h1" stroke="#5393FF" stroke-opacity=".5" opacity="0.5" stroke-width="1.6" '
            'stroke-linecap="round" stroke-linejoin="round"/>'
        )
        self.assertIn("strokeAlpha = 0.25f", out)
        self.assertIn("strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round", out)

    def test_mirror_flag_reaches_the_vector(self):
        self.assertIn("autoMirror = true", kotlin('<path d="M0 0h1" stroke="#5393FF"/>', mirror=True))

    def test_things_a_vector_cannot_draw_fail(self):
        for body in (
            '<path d="M0 0h1" stroke="#5393FF" stroke-dasharray="2" stroke-dashoffset="1"/>',
            '<path d="M0 0h1" stroke="#5393FF" transform="skewX(10)"/>',
            '<rect x="0" y="0" width="4" height="4" rx="1" fill="#5393FF"/>',
            '<image href="x.png"/>',
        ):
            with self.subTest(body=body), self.assertRaises(vector.Unsupported):
                kotlin(body)


if __name__ == "__main__":
    unittest.main()
