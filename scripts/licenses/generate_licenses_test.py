"""generate-licenses.py 的用例：许可表本身自洽，缺正文、缺许可、APPROVED 之外都会失败，兜底只认 WITH 前的 id。"""

from __future__ import annotations

import hashlib
import importlib.util
import io
import json
import os
import tarfile
import sys
import unittest
import unittest.mock
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "generate-licenses.py"
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("generate_licenses", SCRIPT)
licenses = importlib.util.module_from_spec(spec)
spec.loader.exec_module(licenses)


class ApprovedTableTest(unittest.TestCase):
    def test_approved_matches_cargo_about(self):
        self.assertEqual(licenses.accepted(), licenses.approved_ids())

    def test_every_approved_license_has_text(self):
        for license_id in licenses.approved_ids():
            with self.subTest(license_id=license_id):
                self.assertGreater(len(licenses.standard_text(license_id).strip()), 150)

    def test_with_expression_uses_base_text(self):
        self.assertEqual(licenses.standard_text("Apache-2.0 WITH LLVM-exception"), licenses.standard_text("Apache-2.0"))

    def test_unknown_license_has_no_text(self):
        with self.assertRaises(licenses.GenerateError):
            licenses.standard_text("GPL-3.0-only")


class AssetTest(unittest.TestCase):
    def test_each_supported_platform_has_its_asset(self):
        self.assertEqual(licenses.cargo_about_asset("linux", "x86_64")[0], "x86_64-unknown-linux-musl")
        self.assertEqual(licenses.cargo_about_asset("darwin", "arm64")[0], "aarch64-apple-darwin")

    def test_sha256_is_pinned_per_asset(self):
        self.assertEqual(
            licenses.cargo_about_asset("darwin", "arm64")[1],
            "45520df8680978a6e7c4023ace0e3f311c73efc208cd68e5a7f4ed4822fa0349",
        )
        digests = {sha for _, sha in licenses.CARGO_ABOUT_ASSETS.values()}
        self.assertEqual(len(digests), len(licenses.CARGO_ABOUT_ASSETS))

    def test_unknown_platform_names_the_triple(self):
        with self.assertRaises(licenses.GenerateError) as raised:
            licenses.cargo_about_asset("darwin", "x86_64")
        self.assertIn("darwin/x86_64", str(raised.exception))


class CacheTest(unittest.TestCase):
    def test_cache_from_another_platform_is_not_used(self):
        root = Path(self.enterContext(__import__("tempfile").TemporaryDirectory()))
        stale = root / "wonton" / "cargo-about" / licenses.CARGO_ABOUT_VERSION / "aarch64-apple-darwin" / "cargo-about"
        stale.parent.mkdir(parents=True)
        stale.write_bytes(b"ELF-from-a-mac-cache")

        payload = io.BytesIO()
        with tarfile.open(fileobj=payload, mode="w:gz") as tar:
            data = b"#!/bin/sh\necho cargo-about 0.9.3\n"
            info = tarfile.TarInfo(f"cargo-about-{licenses.CARGO_ABOUT_VERSION}-x86_64-unknown-linux-musl/cargo-about")
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
        archive = payload.getvalue()
        sha = hashlib.sha256(archive).hexdigest()
        triple = "x86_64-unknown-linux-musl"

        class Response:
            def __enter__(self):
                return self

            def __exit__(self, *args):
                return False

            def read(self):
                return archive

        with (
            unittest.mock.patch.dict(os.environ, {"XDG_CACHE_HOME": str(root)}),
            unittest.mock.patch.object(licenses.shutil, "which", return_value=None),
            unittest.mock.patch.object(licenses, "cargo_about_asset", return_value=(triple, sha)),
            unittest.mock.patch.object(licenses.urllib.request, "urlopen", return_value=Response()),
        ):
            path = licenses.cargo_about_path()
        self.assertEqual(path, root / "wonton" / "cargo-about" / licenses.CARGO_ABOUT_VERSION / triple / "cargo-about")
        self.assertTrue(path.read_bytes().startswith(b"#!/bin/sh"))

class RustFallbackTest(unittest.TestCase):
    def test_crate_without_license_file_uses_declared_license(self):
        payload = {
            "overview": [],
            "crates": [{"package": {"name": "opaque-debug", "version": "1"}, "license": "MIT OR Apache-2.0"}],
        }
        stub = Path(self.enterContext(__import__("tempfile").TemporaryDirectory())) / "cargo-about"
        stub.write_text(f"#!/usr/bin/env python3\nprint({json.dumps(json.dumps(payload))})\n", encoding="utf-8")
        stub.chmod(0o755)
        with unittest.mock.patch.object(licenses, "cargo_about_path", return_value=stub):
            components, texts = licenses.rust_components("ios")
        self.assertEqual(components[0]["license_ids"], ["MIT"])
        self.assertIn("Permission is hereby granted", texts["MIT"]["text"])


class AndroidTest(unittest.TestCase):
    def export(self, *libraries: dict) -> Path:
        path = Path(self.enterContext(__import__("tempfile").TemporaryDirectory())) / "aboutlibraries.json"
        path.write_text(
            json.dumps(
                {
                    "licenses": {"MIT": {"name": "MIT License", "text": ""}},
                    "libraries": list(libraries),
                }
            ),
            encoding="utf-8",
        )
        return path

    def test_empty_license_text_falls_back_to_standard_text(self):
        path = self.export({"uniqueId": "x:y", "licenses": ["MIT"]})
        components, texts = licenses.android_components(path)
        self.assertEqual(components[0]["license_ids"], ["MIT"])
        self.assertIn("Permission is hereby granted", texts["MIT"]["text"])

    def test_unknown_library_without_license_fails(self):
        path = self.export({"uniqueId": "x:y"})
        with self.assertRaises(licenses.GenerateError):
            licenses.android_components(path)

    def test_only_rust_shim_may_omit_license(self):
        path = self.export({"uniqueId": "org.rustls:rustls-platform-verifier"})
        components, _ = licenses.android_components(path)
        self.assertEqual(components[0]["license_ids"], ["Apache-2.0", "MIT"])


class VerifyTest(unittest.TestCase):
    def check(self, data: dict, platform: str = "ios") -> None:
        path = Path(self.enterContext(__import__("tempfile").TemporaryDirectory())) / "licenses.json"
        path.write_text(json.dumps(data), encoding="utf-8")
        licenses.verify(path, platform)

    def payload(self, **overrides) -> dict:
        data = {
            "components": [
                {"source": "rust", "name": "serde", "version": "1", "license": "MIT", "license_ids": ["MIT"]}
            ],
            "licenses": {"MIT": {"name": "MIT", "text": "x" * 200}},
        }
        data.update(overrides)
        return data

    def test_passes(self):
        self.check(self.payload())

    def test_missing_sentinel_fails(self):
        data = self.payload()
        data["components"][0]["name"] = "other"
        with self.assertRaises(licenses.GenerateError):
            self.check(data)

    def test_license_outside_approved_fails(self):
        data = self.payload()
        data["licenses"]["GPL-3.0-only"] = {"name": "GPL", "text": "x" * 200}
        data["components"][0]["license_ids"] = ["MIT", "GPL-3.0-only"]
        with self.assertRaises(licenses.GenerateError):
            self.check(data)

    def test_short_text_fails(self):
        data = self.payload()
        data["licenses"]["MIT"]["text"] = "short"
        with self.assertRaises(licenses.GenerateError):
            self.check(data)


if __name__ == "__main__":
    unittest.main()
