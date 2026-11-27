"""generate-design.py 图标部分的用例：换色按配比走，画不了的 SVG、不够的对比度、被改过的上游原件都会失败。"""

from __future__ import annotations

import base64
import hashlib
import importlib.util
import io
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "generate-design.py"
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("generate_design", SCRIPT)
design = importlib.util.module_from_spec(spec)
spec.loader.exec_module(design)

BASE = design.hex_rgb("#5393FF")
RED = design.hex_rgb("#DA1F25")
SVG = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 36 36" fill="none">{}</svg>'


class RecolorTest(unittest.TestCase):
    def test_base_becomes_accent(self):
        self.assertEqual(design.rgb_hex(design.kubai_recolor(BASE, BASE, RED)), "#DA1F25")

    def test_tint_keeps_its_share_of_white(self):
        half = tuple((channel + 255) / 2 for channel in BASE)
        expected = tuple((channel + 255) / 2 for channel in RED)
        for got, want in zip(design.kubai_recolor(half, BASE, RED), expected):
            self.assertAlmostEqual(got, want, delta=1)

    def test_dark_pages_dim_the_white_in_tints_only(self):
        half = tuple((channel + 255) / 2 for channel in BASE)
        self.assertLess(sum(design.kubai_recolor(half, BASE, RED, 0.8)), sum(design.kubai_recolor(half, BASE, RED)))
        self.assertEqual(design.kubai_recolor((255, 255, 255), BASE, RED, 0.8), (255, 255, 255))

    def test_grays_are_left_alone(self):
        for gray in ("#FFFFFF", "#000000", "#33363D"):
            self.assertEqual(design.kubai_recolor(design.hex_rgb(gray), BASE, RED), design.hex_rgb(gray))

    def test_deep_stroke_follows_the_accent_hue(self):
        red, green, blue = design.kubai_recolor(design.hex_rgb("#4262CA"), BASE, RED)
        self.assertGreater(red, blue)
        self.assertLess(red + green + blue, sum(RED))

    def test_svg_paints_and_shadow_matrix_are_recolored(self):
        svg = SVG.format(
            '<path d="M0 0H1" stroke="#5393FF"/>'
            '<feColorMatrix type="matrix" values="0 0 0 0 0.3255 0 0 0 0 0.5765 0 0 0 0 1 0 0 0 0.21 0"/>'
        )
        out = design.kubai_recolored(svg, BASE, RED, 1)
        self.assertIn('stroke="#DA1F25"', out)
        self.assertNotIn("0.3255", out)
        self.assertIn("0.21 0", out)


class PlainSvgTest(unittest.TestCase):
    def rejects(self, body):
        with self.assertRaises(SystemExit):
            design.check_plain_svg("x", SVG.format(body))

    def test_plain_shapes_pass(self):
        design.check_plain_svg("x", SVG.format('<path d="M0 0H1" stroke="#DA1F25" fill="none"/>'))

    def test_css_colors_fail(self):
        self.rejects('<path d="M0 0" fill="color-mix(in srgb, #fff 50%, #000)"/>')
        self.rejects('<path d="M0 0" stroke="var(--accent)"/>')
        self.rejects('<path d="M0 0" fill="currentColor"/>')

    def test_css_outside_paint_attributes_fails(self):
        self.rejects('<path d="M0 0" stroke="#000000" stroke-width="var(--w)"/>')
        self.rejects('<path d="M0 0" opacity="calc(1 - 0.5)"/>')

    def test_style_and_unknown_content_fail(self):
        self.rejects('<path d="M0 0" style="fill:#000"/>')
        self.rejects("<style>path{fill:red}</style>")
        self.rejects('<use href="#a"/>')

    def test_attribute_spellings_the_recolor_would_miss_fail(self):
        self.rejects("<path d='M0 0' fill='#5393FF'/>")
        self.rejects('<path d="M0 0" fill = "#5393FF"/>')

    def test_broken_xml_fails(self):
        with self.assertRaises(SystemExit):
            design.check_plain_svg("x", "<svg><path></svg>")


class PaletteTest(unittest.TestCase):
    def test_enough_contrast_passes(self):
        design.check_palette({"blue": {"light": "#0D66F3", "dark": "#5794F7"}})

    def test_too_pale_on_light_pages_fails(self):
        with self.assertRaises(SystemExit):
            design.check_palette({"pink": {"light": "#FEEBEA", "dark": "#5794F7"}})

    def test_too_dark_on_dark_pages_fails(self):
        with self.assertRaises(SystemExit):
            design.check_palette({"rose": {"light": "#0D66F3", "dark": "#9B494A"}})

    def test_too_dark_on_the_black_emphasis_fill_fails(self):
        # 对白底够，对浅色下黑的强调填充不够。
        with self.assertRaises(SystemExit):
            design.check_palette({"navy": {"light": "#1F2A44", "dark": "#5794F7"}})

    def test_too_dark_on_the_gray_emphasis_fill_fails(self):
        # 对深色页面底够，对深色下 #454545 的强调填充不够。
        with self.assertRaises(SystemExit):
            design.check_palette({"blue": {"light": "#0D66F3", "dark": "#2D7AF5"}})


class BareTest(unittest.TestCase):
    def test_keeps_open_strokes_in_the_base_color(self):
        svg = SVG.format(
            '<defs><mask id="m"><path d="M1 1H2"/></mask></defs>'
            '<path d="M0 0A1 1 0 1 0 2 0z" fill="#93BBFF"/>'
            '<path fill="#93BBFF"/>'
            '<path d="M1 1L2 2" stroke="#FFFFFF"/>'
        )
        out = design.kubai_bare(svg, "#5393FF")
        self.assertEqual(out.count("<path"), 1)
        self.assertIn('stroke="#5393FF"', out)
        self.assertNotIn("<defs", out)

    def test_nothing_left_fails(self):
        with self.assertRaises(SystemExit):
            design.kubai_bare(SVG.format('<path d="M0 0A1 1 0 1 0 2 0Z" fill="#93BBFF"/>'), "#5393FF")


class MonoTest(unittest.TestCase):
    def test_white_knocks_out_what_was_drawn_before_and_shadows_go(self):
        svg = SVG.format(
            '<defs><filter id="sh"><feOffset dy="2"/></filter></defs>'
            '<g filter="url(#sh)"><path d="M0 0H9V9H0Z" fill="#93BBFF"/></g>'
            '<path d="M2 2H4" stroke="#FFFFFF"/>'
        )
        out = design.kubai_mono("x", svg, BASE)
        design.check_plain_svg("x", out)
        self.assertNotIn("filter", out)
        self.assertNotIn("FFFFFF", out)
        self.assertIn('mask="url(#ko-x-0)"', out)
        self.assertIn('fill="#000000" fill-opacity=', out)


class UpstreamFilesTest(unittest.TestCase):
    def folder(self, files: dict[str, bytes]) -> Path:
        root = Path(self.enterContext(tempfile.TemporaryDirectory())) / "kubai"
        sums = []
        for path, content in files.items():
            (root / path).parent.mkdir(parents=True, exist_ok=True)
            (root / path).write_bytes(content)
            sums.append(f"{hashlib.sha256(content).hexdigest()}  {path}")
        (root / "SHA256SUMS").write_text("\n".join(sums) + "\n")
        return root

    def test_matching_files_pass(self):
        root = self.folder({"LICENSE": b"MIT", "svg/a.svg": b"<svg/>"})
        design.check_kubai_files(root, {"LICENSE", "svg/a.svg"})

    def test_edited_svg_fails(self):
        root = self.folder({"LICENSE": b"MIT", "svg/a.svg": b"<svg/>"})
        (root / "svg/a.svg").write_bytes(b"<svg></svg>")
        with self.assertRaises(SystemExit):
            design.check_kubai_files(root, {"LICENSE", "svg/a.svg"})

    def test_edited_license_fails(self):
        root = self.folder({"LICENSE": b"MIT", "svg/a.svg": b"<svg/>"})
        (root / "LICENSE").write_bytes(b"GPL")
        with self.assertRaises(SystemExit):
            design.check_kubai_files(root, {"LICENSE", "svg/a.svg"})

    def test_unregistered_file_fails(self):
        root = self.folder({"LICENSE": b"MIT", "svg/a.svg": b"<svg/>"})
        (root / "svg/extra.svg").write_bytes(b"<svg/>")
        with self.assertRaises(SystemExit):
            design.check_kubai_files(root, {"LICENSE", "svg/a.svg"})

    def test_icon_without_vendored_file_fails(self):
        root = self.folder({"LICENSE": b"MIT", "svg/a.svg": b"<svg/>"})
        with self.assertRaises(SystemExit):
            design.check_kubai_files(root, {"LICENSE", "svg/a.svg", "svg/b.svg"})


class VendorTest(unittest.TestCase):
    def tarball(self, files: dict[str, bytes]) -> tuple[Path, str]:
        path = Path(self.enterContext(tempfile.TemporaryDirectory())) / "kubai.tgz"
        with tarfile.open(path, "w:gz") as archive:
            for name, content in files.items():
                info = tarfile.TarInfo(f"package/{name}")
                info.size = len(content)
                archive.addfile(info, io.BytesIO(content))
        digest = base64.b64encode(hashlib.sha512(path.read_bytes()).digest()).decode()
        return path, f"sha512-{digest}"

    def icons(self, integrity: str, *names: str) -> dict:
        return {
            "upstream": {"kubai": {"package": "kubai-icons-static", "version": "0", "integrity": integrity}},
            "icons": {name: {"kubai": name} for name in names},
        }

    def existing(self) -> Path:
        root = Path(self.enterContext(tempfile.TemporaryDirectory())) / "kubai"
        (root / "svg").mkdir(parents=True)
        (root / "svg/old.svg").write_bytes(b"old")
        return root

    def test_replaces_folder_with_what_icons_use(self):
        tarball, integrity = self.tarball({"LICENSE": b"MIT", "svg/a.svg": b"<svg/>", "svg/unused.svg": b"<svg/>"})
        root = self.existing()
        design.vendor_kubai(tarball, self.icons(integrity, "a"), root)
        design.check_kubai_files(root, {"LICENSE", "svg/a.svg"})

    def test_wrong_tarball_leaves_folder_alone(self):
        tarball, _ = self.tarball({"LICENSE": b"MIT", "svg/a.svg": b"<svg/>"})
        root = self.existing()
        with self.assertRaises(SystemExit):
            design.vendor_kubai(tarball, self.icons("sha512-AAAA", "a"), root)
        self.assertEqual((root / "svg/old.svg").read_bytes(), b"old")

    def test_missing_icon_leaves_folder_alone(self):
        tarball, integrity = self.tarball({"LICENSE": b"MIT", "svg/a.svg": b"<svg/>"})
        root = self.existing()
        with self.assertRaises(SystemExit):
            design.vendor_kubai(tarball, self.icons(integrity, "a", "b"), root)
        self.assertEqual((root / "svg/old.svg").read_bytes(), b"old")


if __name__ == "__main__":
    unittest.main()
