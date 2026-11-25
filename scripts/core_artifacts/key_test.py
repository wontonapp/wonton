"""core_artifacts/key.py 的用例：任何一个输入改了键必变，无关文件不改键；同一份内容键稳定，缺输入直接报错。

键算错不会让作业红，而是让壳悄悄用上旧绑定，所以这里逐个输入文件、逐个输入目录和每项配方都验一遍。
运行：python3 -m unittest discover -s scripts/core_artifacts -t scripts（lint 工作流和本地都跑）
"""

from __future__ import annotations

import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "core_artifacts" / "key.py"
sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location("core_artifacts_key", SCRIPT)
key = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(key)

# 和仓库无关的路径，改了不该动键。
UNRELATED = (
    "apps/ios/Wonton/Sources/ContentView.swift",
    "apps/compose/shared/src/androidMain/kotlin/App.kt",
    "docs/adr/0001-x.md",
    "README.md",
    "scripts/generate-design.py",
)


class KeyTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = Path(self._tmp.name)
        # 每台平台都备好它的全部输入文件和各输入目录里的一个文件，另加假工具链和假 SDK。
        for platform, spec in key.PLATFORMS.items():
            for rel in spec["files"]:
                self._write(rel, f"{platform} 的输入 {rel}\n")
            for tree in spec["trees"] + spec["optional_trees"]:
                self._write(f"{tree}/fixture/{platform}-{tree}.txt", "目录里的文件\n")
        for rel in UNRELATED:
            self._write(rel, "无关文件\n")
        # 探针：假 Xcode、假 SDK 版本、假 NDK。
        self.bin = self.root / "fakebin"
        self._write("fakebin/xcodebuild", "#!/bin/sh\necho 'Xcode 27.1'\necho 'Build version 27A1'\n")
        self._write("fakebin/xcrun", "#!/bin/sh\necho '27.1.0'\n")
        (self.bin / "xcodebuild").chmod(0o755)
        (self.bin / "xcrun").chmod(0o755)
        self._write("sdk/ndk/27.0.12077973/source.properties", "Pkg.Desc=Android NDK\nRevision=27.0.12077973\n")
        self.env = dict(os.environ)
        self.env.update(PATH=f"{self.bin}:{self.env['PATH']}", ANDROID_HOME=str(self.root / "sdk"))
        # 真实机器上的 NDK 变量（GitHub 的 ubuntu 镜像会设）会盖过假 SDK，测试里一律指向假 SDK。
        for name in ("ANDROID_NDK_HOME", "ANDROID_NDK_ROOT", "ANDROID_SDK_ROOT"):
            self.env.pop(name, None)

    def _write(self, rel: str, text: str) -> None:
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def _key(self, platform: str, *args: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, SCRIPT, platform, "--root", str(self.root), *args],
            capture_output=True, text=True, env=env or self.env, check=True,
        )

    def _digest(self, platform: str) -> str:
        return self._key(platform).stdout.strip()

    def test_same_content_same_key(self) -> None:
        for platform in key.PLATFORMS:
            with self.subTest(platform=platform):
                self.assertEqual(self._digest(platform), self._digest(platform))

    def test_platforms_differ(self) -> None:
        ios, android = self._digest("ios"), self._digest("android")
        self.assertNotEqual(ios, android)

    def test_every_input_file_changes_key(self) -> None:
        for platform, spec in key.PLATFORMS.items():
            before = self._digest(platform)
            for rel in spec["files"]:
                with self.subTest(platform=platform, file=rel):
                    original = (self.root / rel).read_text(encoding="utf-8")
                    self._write(rel, original + "改一行\n")
                    self.assertNotEqual(before, self._digest(platform))
                    self._write(rel, original)

    def test_file_in_every_input_tree_changes_key(self) -> None:
        for platform, spec in key.PLATFORMS.items():
            before = self._digest(platform)
            for tree in spec["trees"]:
                with self.subTest(platform=platform, tree=tree):
                    rel = f"{tree}/fixture/added-{tree}.rs"
                    self._write(rel, "新文件\n")
                    self.assertNotEqual(before, self._digest(platform))
                    (self.root / rel).unlink()

    def test_removed_input_tree_file_changes_key(self) -> None:
        for platform, spec in key.PLATFORMS.items():
            before = self._digest(platform)
            for tree in spec["trees"]:
                with self.subTest(platform=platform, tree=tree):
                    rel = f"{tree}/fixture/{platform}-{tree}.txt"
                    original = (self.root / rel).read_text(encoding="utf-8")
                    (self.root / rel).unlink()
                    self.assertNotEqual(before, self._digest(platform))
                    self._write(rel, original)

    def test_md_in_tree_keeps_key(self) -> None:
        # 路径过滤排除 .md，键也不该跟着文档变，否则只改文档的合并会让所有壳 PR 失效。
        for platform, spec in key.PLATFORMS.items():
            before = self._digest(platform)
            rel = f"{spec['trees'][0]}/fixture/说明.md"
            self._write(rel, "文档\n")
            self.assertEqual(before, self._digest(platform))
            self._write(rel, "改了文档\n")
            self.assertEqual(before, self._digest(platform))

    def test_absent_optional_tree_is_fine(self) -> None:
        # .cargo 现在仓库里没有：缺了照样算，空目录也算同一份。
        saved = {p: p.read_text(encoding="utf-8") for p in (self.root / ".cargo").rglob("*") if p.is_file()}
        for path in saved:
            path.unlink()
        (self.root / ".cargo/fixture").rmdir()
        (self.root / ".cargo").rmdir()
        absent = {platform: self._digest(platform) for platform in key.PLATFORMS}
        (self.root / ".cargo").mkdir()
        for platform, digest in absent.items():
            self.assertEqual(digest, self._digest(platform))
        for path, body in saved.items():
            self._write(path.relative_to(self.root).as_posix(), body)

    def test_pinned_xcode_changes_ios_key(self) -> None:
        # 钉死版本说了算，机器上默认的 Xcode 是哪个不影响；换了钉的版本必须换键。
        before = self._digest("ios")
        env = dict(self.env, XCODE_VERSION="27.1")
        pinned = self._key("ios", env=env).stdout.strip()
        self.assertNotEqual(before, pinned)
        self.assertNotEqual(pinned, self._key("ios", env=dict(env, XCODE_VERSION="27.2")).stdout.strip())

    def test_pinned_ndk_changes_android_key(self) -> None:
        # 钉死版本说了算，机器上装的是哪个不影响；换了钉的版本必须换键。
        before = self._digest("android")
        env = dict(self.env, ANDROID_NDK_VERSION="28.0.1")
        pinned = self._key("android", env=env).stdout.strip()
        self.assertNotEqual(before, pinned)
        self.assertNotEqual(pinned, self._key("android", env=dict(env, ANDROID_NDK_VERSION="29.0.1")).stdout.strip())

    def test_ios_key_ignores_ndk(self) -> None:
        # iOS 产物和 Android NDK 无关：同一台机器装没装 NDK 不该算出两个键。
        before = self._digest("ios")
        self._write("sdk/ndk/27.0.12077973/source.properties", "Revision=99.0.1\n")
        self.assertEqual(before, self._digest("ios"))

    def test_env_ndk_changes_android_key(self) -> None:
        before = self._digest("android")
        self._write("other-ndk/source.properties", "Revision=28.0.1\n")
        env = dict(self.env, ANDROID_NDK_HOME=str(self.root / "other-ndk"))
        self.assertNotEqual(before, self._key("android", env=env).stdout.strip())

    def test_missing_tree_fails_loudly(self) -> None:
        for platform, spec in key.PLATFORMS.items():
            tree = spec["trees"][0]
            with self.subTest(platform=platform, tree=tree):
                for path in sorted((self.root / tree).rglob("*"), reverse=True):
                    path.unlink() if path.is_file() else path.rmdir()
                (self.root / tree).rmdir()
                done = subprocess.run(
                    [sys.executable, SCRIPT, platform, "--root", str(self.root)],
                    capture_output=True, text=True, env=self.env, check=False,
                )
                self.assertEqual(done.returncode, 2)
                self.assertIn(tree, done.stderr)
                self._write(f"{tree}/fixture/restored.txt", "目录里的文件\n")

    def test_unrelated_files_keep_key(self) -> None:
        for platform in key.PLATFORMS:
            before = self._digest(platform)
            for rel in UNRELATED:
                with self.subTest(platform=platform, file=rel):
                    original = (self.root / rel).read_text(encoding="utf-8")
                    self._write(rel, original + "改一行\n")
                    self.assertEqual(before, self._digest(platform))
                    self._write(rel, original)

    def test_recipe_changes_key(self) -> None:
        for platform in key.PLATFORMS:
            before = self._digest(platform)
            with self.subTest(platform=platform, recipe="profile"):
                self.assertNotEqual(before, self._key(platform, "--profile", "release").stdout.strip())
            for name in key.PLATFORMS[platform]["env"]:
                with self.subTest(platform=platform, env=name):
                    env = dict(self.env)
                    env[name] = "别的值"
                    self.assertNotEqual(before, self._key(platform, env=env).stdout.strip())

    def test_toolchain_probes_change_key(self) -> None:
        for platform, spec in key.PLATFORMS.items():
            before = self._digest(platform)
            for name in spec["probes"]:
                with self.subTest(platform=platform, probe=name):
                    if name == "xcode":
                        self._write("fakebin/xcodebuild", "#!/bin/sh\necho 'Xcode 27.2'\n")
                        self.assertNotEqual(before, self._digest(platform))
                        self._write("fakebin/xcodebuild", "#!/bin/sh\necho 'Xcode 27.1'\necho 'Build version 27A1'\n")

                    elif name == "ndk":
                        ndk = "sdk/ndk/27.0.12077973/source.properties"
                        original = (self.root / ndk).read_text(encoding="utf-8")
                        self._write(ndk, original.replace("Revision=27.0.12077973", "Revision=28.0.1"))
                        self.assertNotEqual(before, self._digest(platform))
                        self._write(ndk, original)

    def test_missing_input_fails_loudly(self) -> None:
        for platform, spec in key.PLATFORMS.items():
            rel = spec["files"][0]
            with self.subTest(platform=platform, file=rel):
                original = (self.root / rel).read_text(encoding="utf-8")
                (self.root / rel).unlink()
                done = subprocess.run(
                    [sys.executable, SCRIPT, platform, "--root", str(self.root)],
                    capture_output=True, text=True, env=self.env, check=False,
                )
                self.assertEqual(done.returncode, 2)
                self.assertIn(rel, done.stderr)
                self._write(rel, original)


if __name__ == "__main__":
    unittest.main()
