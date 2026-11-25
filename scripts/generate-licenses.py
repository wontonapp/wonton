#!/usr/bin/env python3
"""生成 App“开源许可”页要的数据（licenses.json）：Rust crate 和 Android 依赖，各自带上许可全文。

两类数据都来自现成工具，生成物不手改：
  - Rust：cargo-about 从 Cargo.lock 的依赖图取 crate 和许可文件（版本、URL、sha256 钉死在下面）。
  - Android：Gradle 的 AboutLibraries 插件在构建时收集，本脚本读它导出的 JSON。

用法：
  scripts/generate-licenses.py --platform android --out build/licenses
  scripts/generate-licenses.py --platform ios --out Wonton/Generated

输出目录里只写 licenses.json；写进 App 的资源由调用方接（Gradle 的 generated assets、Xcode 的构建阶段）。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
APPROVED_FILE = ROOT / "licenses" / "APPROVED"
TEXTS = ROOT / "licenses" / "texts"
CARGO_ABOUT_CONFIG = ROOT / "licenses" / "about.toml"

# cargo-about 是开发期工具，不进 App 二进制；它是 MIT OR Apache-2.0，升级时改这里并跑一遍 check-licenses.sh。
# 发布资产按平台分开（0.9.3 没有 x86_64 darwin 的），每个钉自己的 sha256；第二段是 tar 里的目录名。
CARGO_ABOUT_VERSION = "0.9.3"
CARGO_ABOUT_ASSETS = {
    ("linux", "x86_64"): (
        "x86_64-unknown-linux-musl",
        "3ad1e94ea1e63d69d1961f4fb15ccbbe9575d906111f96d2cb4dce6df48f71b4",
    ),
    ("darwin", "arm64"): (
        "aarch64-apple-darwin",
        "45520df8680978a6e7c4023ace0e3f311c73efc208cd68e5a7f4ed4822fa0349",
    ),
}

# 各平台真正编进 App 的目标。一次只给一个平台，页面就不会列出另一端的 crate。
TARGETS = {
    "android": ["aarch64-linux-android"],
    "ios": ["aarch64-apple-ios", "aarch64-apple-ios-sim"],
}

# 只在 POM 里没写许可的 Android 组件（本地发布的 Rust JNI 构件），按上游的许可补。
ANDROID_EXTRA_LICENSES = {
    "org.rustls:rustls-platform-verifier": ["MIT", "Apache-2.0"],
}

# Android 依赖里可以拿来当“清单真的收了东西”的证据；任何一个缺失都说明 Gradle 那边的收集断了。
ANDROID_SENTINELS = ["androidx.activity:activity", "androidx.core:core"]


class GenerateError(Exception):
    pass


def accepted() -> list[str]:
    """about.toml 的 accepted，顺序即优先级。"""
    text = CARGO_ABOUT_CONFIG.read_text(encoding="utf-8")
    body = text.split("accepted = [", 1)[1].split("]", 1)[0]
    lines = [line.split("#", 1)[0] for line in body.splitlines()]
    return [
        part.strip().strip('"').removeprefix("LicenseRef-")
        for part in "\n".join(lines).replace("\n", " ").split(",")
        if part.strip()
    ]


def approved_ids() -> list[str]:
    """licenses/APPROVED 里的许可 id，按文件顺序（跳过表头那行）。"""
    return [
        line.split("|", 1)[0].strip()
        for line in APPROVED_FILE.read_text(encoding="utf-8").splitlines()
        if "|" in line and not line.startswith("格式")
    ]


def standard_text(license_id: str) -> str:
    """许可全文兜底：依赖只声明了 SPDX id、没带正文时用。"""
    path = TEXTS / f"{license_id.split(' WITH ')[0]}.txt"
    if not path.is_file():
        raise GenerateError(f"licenses/texts 里没有 {license_id} 的全文")
    return path.read_text(encoding="utf-8")


def cargo_about_asset(system: str | None = None, machine: str | None = None) -> tuple[str, str]:
    """这台机器该下载的资产目录名和它的 sha256；没有对应资产时报清楚平台。"""
    system = system or sys.platform
    machine = machine or platform.machine()
    found = CARGO_ABOUT_ASSETS.get((system, machine))
    if found:
        return found
    raise GenerateError(
        f"没有 cargo-about {CARGO_ABOUT_VERSION} 的 {system}/{machine} 资产；"
        f"先 cargo install cargo-about --version {CARGO_ABOUT_VERSION} --locked 放进 PATH"
    )


def cargo_about_path() -> Path:
    """找 cargo-about：PATH 里有就用，没有就下载钉死的那份到缓存目录，校验 sha256。"""
    found = shutil.which("cargo-about")
    if found:
        version = subprocess.run([found, "--version"], capture_output=True, text=True).stdout
        if CARGO_ABOUT_VERSION in version:
            return Path(found)
        raise GenerateError(f"PATH 里的 cargo-about 是 {version.strip()}，要 {CARGO_ABOUT_VERSION}")

    # 缓存按平台分开：以前只按版本存，mac 上会拿回之前在别的平台下的那份（以前下过 Linux 版就是 ELF）。
    triple, sha256 = cargo_about_asset()
    cache = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "wonton" / "cargo-about"
    binary = cache / CARGO_ABOUT_VERSION / triple / "cargo-about"
    if binary.is_file():
        return binary

    url = (
        "https://github.com/EmbarkStudios/cargo-about/releases/download/"
        f"{CARGO_ABOUT_VERSION}/cargo-about-{CARGO_ABOUT_VERSION}-{triple}.tar.gz"
    )
    print(f"下载 cargo-about {CARGO_ABOUT_VERSION}（{triple}）", file=sys.stderr)
    with urllib.request.urlopen(url, timeout=120) as response:
        archive = response.read()
    digest = hashlib.sha256(archive).hexdigest()
    if digest != sha256:
        raise GenerateError(f"cargo-about 的 sha256 是 {digest}，和钉死的对不上")
    binary.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        subprocess.run(["tar", "xzf", "-", "-C", tmp], input=archive, check=True)
        inner = Path(tmp) / f"cargo-about-{CARGO_ABOUT_VERSION}-{triple}" / "cargo-about"
        binary.write_bytes(inner.read_bytes())
    binary.chmod(0o755)
    return binary


def rust_components(platform: str) -> tuple[list[dict], dict[str, dict]]:
    """跑 cargo-about 取 Rust crate。它按 accepted 的优先级挑一个许可，挑不到就报错（--fail）。"""
    with tempfile.NamedTemporaryFile("w", suffix=".toml", delete=False) as config:
        config.write(CARGO_ABOUT_CONFIG.read_text(encoding="utf-8"))
        config_path = Path(config.name)
    targets = TARGETS[platform]
    command = [
        str(cargo_about_path()),
        "generate",
        "--format",
        "json",
        "--config",
        str(config_path),
        "--manifest-path",
        str(ROOT / "crates" / "wonton-client" / "Cargo.toml"),
        "--locked",
        "--fail",
    ]
    for target in targets:
        command += ["--target", target]
    try:
        run = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    finally:
        config_path.unlink()
    if run.returncode != 0:
        raise GenerateError(f"cargo-about 失败：\n{run.stderr.strip()}")

    raw = json.loads(run.stdout)
    texts = {entry["id"]: entry for entry in raw["overview"]}
    # overview 是“许可 → 用到它的 crate 下标”，反过来换成“crate 下标 → 许可”。
    used: dict[int, list[str]] = {}
    for license_id, entry in texts.items():
        for index in entry["indices"]:
            used.setdefault(index, []).append(license_id)

    components, licenses = [], {}
    for index, crate in enumerate(raw["crates"]):
        ids = sorted(used.get(index, []))
        if not ids:
            # crate 没带许可文件（如 opaque-debug）：按它自己声明的表达式挑第一个接受的许可。
            declared = {part.strip() for part in crate["license"].replace("(", "").replace(")", "").split(" OR ") for part in part.split(" AND ")}
            ids = [license_id for license_id in accepted() if license_id in declared][:1]
            if not ids:
                raise GenerateError(f"{crate['package']['name']} 没有对应的许可文本，声明是 {crate['license']}")
            licenses[ids[0]] = {"name": ids[0], "text": standard_text(ids[0])}
        for license_id in ids:
            entry = texts.get(license_id)
            if entry is None:
                # 兜底分支刚补过正文，这里不覆盖它。
                continue
            if not entry["text"].strip():
                raise GenerateError(f"许可 {license_id} 没有文本")
            licenses.setdefault(license_id, {"name": entry["name"], "text": entry["text"]})
        components.append(
            {
                "source": "rust",
                "name": crate["package"]["name"],
                "version": crate["package"]["version"],
                "license": crate["license"],
                "license_ids": ids,
            }
        )
    return components, licenses


def android_components(export: Path) -> tuple[list[dict], dict[str, dict]]:
    """读 AboutLibraries 导出的 JSON。它已经带好每个依赖的许可 id 和全文，不用再抓。"""
    raw = json.loads(export.read_text(encoding="utf-8"))
    licenses = {
        license_id: {"name": entry.get("name") or license_id, "text": entry.get("text") or ""}
        for license_id, entry in raw["licenses"].items()
    }
    components = []
    for library in raw["libraries"]:
        ids = sorted(library.get("licenses") or ANDROID_EXTRA_LICENSES.get(library["uniqueId"], []))
        if not ids:
            raise GenerateError(f"{library['uniqueId']} 没有许可")
        for license_id in ids:
            if not licenses.get(license_id, {}).get("text", "").strip():
                licenses[license_id] = {"name": license_id, "text": standard_text(license_id)}
        components.append(
            {
                "source": "android",
                "name": library["uniqueId"],
                "version": library.get("artifactVersion", ""),
                "license": " AND ".join(ids),
                "license_ids": ids,
            }
        )
    return components, licenses


RUST_SENTINELS = ["serde"]


def verify(target: Path, platform: str) -> None:
    """结构检查：清单能读、每个组件都有许可、每个许可都在 APPROVED 里且有全文、哨兵依赖在。"""
    data = json.loads(target.read_text(encoding="utf-8"))
    approved = set(approved_ids())
    problems = []
    names = {component["name"] for component in data["components"]}
    sentinels = ANDROID_SENTINELS if platform == "android" else RUST_SENTINELS
    problems += [f"清单里没有 {name}" for name in sentinels if name not in names]
    for component in data["components"]:
        if not component["license_ids"]:
            problems.append(f"{component['name']} 没有许可")
            continue
        for license_id in component["license_ids"]:
            if license_id not in approved:
                problems.append(f"{component['name']} 用了 APPROVED 之外的 {license_id}")
            if len(data["licenses"].get(license_id, {}).get("text", "").strip()) < 150:
                problems.append(f"{license_id} 没有全文")
    if problems:
        raise GenerateError("清单不完整：\n  " + "\n  ".join(sorted(set(problems))))
    print(f"检查通过：{len(data['components'])} 个组件、{len(data['licenses'])} 种许可")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", required=True, choices=sorted(TARGETS))
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--verify", action="store_true", help="写完后再做一遍结构检查")
    parser.add_argument(
        "--android-export",
        type=Path,
        help="AboutLibraries 导出的 aboutlibraries.json，Android 构建必给",
    )
    args = parser.parse_args()

    if args.platform == "android":
        if not args.android_export or not args.android_export.is_file():
            raise GenerateError("Android 要 --android-export，且文件必须存在")
        components, licenses = android_components(args.android_export)
        missing = [name for name in ANDROID_SENTINELS if name not in {c["name"] for c in components}]
        if missing:
            raise GenerateError(f"AboutLibraries 的清单里没有 {', '.join(missing)}，收集断了")
    else:
        components, licenses = rust_components(args.platform)

    components.sort(key=lambda component: (component["name"].lower(), component["version"]))
    if not components:
        raise GenerateError("一个组件都没收集到")
    tool = "AboutLibraries" if args.platform == "android" else f"cargo-about {CARGO_ABOUT_VERSION}"
    data = {
        "generated_with": tool,
        "components": components,
        "licenses": dict(sorted(licenses.items())),
    }

    args.out.mkdir(parents=True, exist_ok=True)
    target = args.out / "licenses.json"
    target.write_text(json.dumps(data, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"{len(components)} 个组件、{len(licenses)} 种许可 → {target}")
    if args.verify:
        verify(target, args.platform)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except GenerateError as error:
        print(f"生成许可清单失败：{error}", file=sys.stderr)
        sys.exit(1)
