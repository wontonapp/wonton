#!/usr/bin/env python3
"""核心产物的内容哈希键：改了任何一个输入，键就变。

iOS 的作业和 Android 的作业都先按键去 main 上找同一份核心源码编好的产物（XCFramework 和 Swift 绑定、.so 和
Kotlin 绑定），命中就跳过编译。键算错的后果不是作业红，而是壳悄悄链接上对的接口之前的旧绑定，所以这里按“会影响
产物的所有输入”取：核心源码和它们的清单、锁文件、工具链文件、生成绑定的脚本、构建配方（工作流里的命令和参数也算）、
探针取到的 Xcode/模拟器 SDK 版本和工作流里钉死的 NDK 版本、以及这个脚本自己。规则和命中方式写在 .github/workflows/README.md 的“核心产物”一节。

用法：python3 scripts/core_artifacts/key.py <ios|android> [--profile debug] [--root 仓库根]
只往 stdout 打键；缺输入（文件或整棵树）时打到 stderr 并以 2 退出，别让它悄悄少算一大块。
"""

from __future__ import annotations

import argparse
import hashlib
import os
import subprocess
import sys
from pathlib import Path

# 每项都是会改变产物、或会让旧产物对不上的东西。目录整个算进去（宁可多失效，不要漏）：crates/ 里有源码、
# uniffi.toml 和清单；vendor/ 是打了补丁的上游代码。.md 不算：改文档不会变产物，路径过滤也排除它。
# 工作流自己也算：产物是怎么编的（cargo-ndk 版本、ARCHS、-P 参数、哪个镜像）写在那里，改了指令而键不变最危险。
PLATFORMS = {
    "ios": {
        "files": (
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "apps/ios/scripts/build-core.sh",
            "apps/ios/scripts/generate-swift-bindings.sh",
            ".github/workflows/ios.yml",
            ".github/workflows/cache.yml",
        ),
        "trees": ("crates", "vendor"),
        "optional_trees": (".cargo",),
        "env": (
            "WONTON_IOS_TARGETS",
            # build-core.sh 自己读它（case 里按 Debug 开关 ui-fake）。
            "WONTON_UI_FAKE",
            "CARGO_INCREMENTAL",
            "CARGO_PROFILE_DEV_DEBUG",
            "CARGO_PROFILE_RELEASE_DEBUG",
        ),
        # 只有 Xcode：产物和 Android NDK 无关，别让它跟着机器类别变。iOS SDK 跟着 Xcode 走，单独探会随镜像补丁抖。
        "probes": {"xcode": None},
    },
    "android": {
        "files": (
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            "apps/compose/scripts/build-core.sh",
            "apps/compose/shared/build.gradle.kts",
            # ABI、构建类型、假数据入口的默认值和 minSdk（minSdk 传给 cargo-ndk，改 .so 的 API 下限）。
            "apps/compose/gradle.properties",
            "apps/compose/gradle/libs.versions.toml",
            ".github/workflows/android.yml",
            ".github/workflows/cache.yml",
        ),
        "trees": ("crates", "vendor"),
        "optional_trees": (".cargo",),
        "env": (
            "CARGO_INCREMENTAL",
            "CARGO_PROFILE_DEV_DEBUG",
            "CARGO_PROFILE_RELEASE_DEBUG",
        ),
        "probes": {"ndk": None},
    },
}

# 这个脚本自己：改了算法或输入清单，旧键的产物也就不是“同一个东西”了。
SELF = Path(__file__).resolve()


def xcode_version() -> str:
    """产物用的 Xcode：CI 里由工作流钉死的 `XCODE_VERSION` 说了算，别去探机器上是哪个补丁版本。

    和 NDK 同理：镜像/机器上默认的 Xcode 逐台不同，探它会算出两个键。本地没设这个变量时才跑 `xcodebuild -version`。
    """
    pinned = os.environ.get("XCODE_VERSION", "").strip()
    return pinned if pinned else probe(("xcodebuild", "-version"))


def probe(cmd: tuple[str, ...]) -> str:
    """跑一条探针命令，只取第一行；没有这个工具时写 unset，跑挂了写退出码、超时写 timeout（都和正常值不同）。"""
    try:
        done = subprocess.run(cmd, capture_output=True, text=True, timeout=60, check=False)
    except FileNotFoundError:
        return "unset"
    except subprocess.TimeoutExpired:
        print(f"::warning::探针超时：{' '.join(cmd)}", file=sys.stderr)
        return "timeout"
    if done.returncode != 0:
        return f"error {done.returncode}"
    lines = done.stdout.strip().splitlines()
    return lines[0] if lines else "empty"


def ndk_revision() -> str:
    """产物用的 NDK 版本。

    CI 里由工作流钉死的 `ANDROID_NDK_VERSION` 说了算：镜像自带哪个 NDK 逐台不同，探机器上装的那个会让同一份
    源码在两台 runner 上算出两个键、说好的命中变成现编。本地没设这个变量时才退回探 SDK 里最新的那份。
    """
    pinned = os.environ.get("ANDROID_NDK_VERSION", "").strip()
    if pinned:
        return pinned
    ndk = os.environ.get("ANDROID_NDK_HOME") or os.environ.get("ANDROID_NDK_ROOT")
    if not ndk:
        sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
        # 和 build-core.sh 的 sort -V、cargo-ndk 一样按版本号挑最新的，不按字典序（27.9 和 27.10）。
        found = sorted(
            ((Path(sdk) / "ndk").glob("*") if sdk else []),
            key=lambda path: [int(part) for part in path.name.split(".") if part.isdigit()],
        )
        ndk = str(found[-1]) if found else ""
    if not ndk:
        return "unset"
    properties = Path(ndk) / "source.properties"
    if not properties.is_file():
        return "unset"
    for line in properties.read_text(encoding="utf-8", errors="replace").splitlines():
        if line.startswith("Revision="):
            return line.partition("=")[2].strip()
    return f"unset {ndk}"


def probe_values(platform: str) -> list[tuple[str, str]]:
    return [
        (name, (ndk_revision() if name == "ndk" else xcode_version()) if cmd is None else probe(cmd))
        for name, cmd in PLATFORMS[platform]["probes"].items()
    ]


def inputs(root: Path, platform: str) -> list[str]:
    spec = PLATFORMS[platform]
    missing = [rel for rel in spec["files"] if not (root / rel).is_file()]
    missing += [tree for tree in spec["trees"] if not (root / tree).is_dir()]
    if missing:
        print(f"缺输入：{'、'.join(missing)}（仓库根 {root}）", file=sys.stderr)
        raise SystemExit(2)
    entries = list(spec["files"])
    for tree in spec["trees"] + spec["optional_trees"]:
        entries.extend(
            path.relative_to(root).as_posix()
            for path in (root / tree).rglob("*")
            if path.is_file() and path.suffix != ".md"
        )
    return sorted(entries)


def digest(root: Path, platform: str, profile: str) -> str:
    spec = PLATFORMS[platform]
    sha = hashlib.sha256(b"wonton-core-artifact v1\0")
    sha.update(platform.encode() + b"\0")
    for rel in inputs(root, platform):
        sha.update(rel.encode() + b"\0")
        with open(root / rel, "rb") as handle:
            for chunk in iter(lambda: handle.read(1 << 20), b""):
                sha.update(chunk)
        sha.update(b"\0")
    sha.update(f"profile={profile}\0".encode())
    for name in spec["env"]:
        sha.update(f"env {name}={os.environ.get(name, '')}\0".encode())
    for name, value in probe_values(platform):
        sha.update(f"probe {name}={value}\0".encode())
    sha.update(SELF.read_bytes())
    return sha.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("platform", choices=sorted(PLATFORMS))
    parser.add_argument("--profile", default="debug")
    parser.add_argument("--root", type=Path, help="仓库根，默认为这个脚本所在的仓库")
    args = parser.parse_args()
    root = args.root or Path(
        subprocess.run(
            ["git", "-C", SELF.parent, "rev-parse", "--show-toplevel"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
    )
    print(digest(root.resolve(), args.platform, args.profile))


if __name__ == "__main__":
    main()
