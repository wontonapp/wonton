#!/usr/bin/env python3
"""夜跑失败上报：按失败的测试开 issue；同一条测试已经有开着的 issue 就在上面留言。

两个端用同一份：iOS 从 .xcresult 里取失败的用例，Android 从 JUnit XML 里取。没有解析到失败用例
（构建或前置步骤炸了）时报一条作业级 issue。`GH_TOKEN` 由工作流给，仓库用 `--repo` 传。
"""

import argparse
import json
import os
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

TITLE_PREFIX = "夜跑失败："


def ios_failures(bundle: str) -> list[str]:
    out = subprocess.run(
        ["xcrun", "xcresulttool", "get", "test-results", "tests", "--path", bundle, "--format", "json"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    failures: list[str] = []

    def walk(node: dict, suites: list[str]) -> None:
        node_type = node.get("nodeType")
        name = node.get("name", "")
        if node_type == "Test Case":
            if node.get("result") == "Failed":
                failures.append(".".join([*suites, name.removesuffix("()")]))
            return
        next_suites = [*suites, name] if node_type in ("Test Suite", "Unit test bundle") and name else suites
        for child in node.get("children", []):
            walk(child, next_suites)

    for node in json.loads(out).get("testNodes", []):
        walk(node, [])
    return failures


def android_failures(results_dir: str) -> list[str]:
    failures: list[str] = []
    for path in Path(results_dir).rglob("TEST-*.xml"):
        for case in ET.parse(path).getroot().iter("testcase"):
            if case.find("failure") is None and case.find("error") is None:
                continue
            suite = case.get("classname", "").rsplit(".", 1)[-1]
            failures.append(f"{suite}.{case.get('name', '')}")
    return failures


def gh(*args: str) -> str:
    return subprocess.run(["gh", *args], check=True, capture_output=True, text=True).stdout


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", required=True)
    parser.add_argument("--platform", required=True, choices=["ios", "android"])
    parser.add_argument("--run-url", required=True)
    parser.add_argument("--xcresult")
    parser.add_argument("--results-dir")
    args = parser.parse_args()

    if args.platform == "ios":
        try:
            failures = ios_failures(args.xcresult) if args.xcresult and Path(args.xcresult).exists() else []
        except (subprocess.CalledProcessError, json.JSONDecodeError):
            failures = []
    else:
        try:
            failures = android_failures(args.results_dir) if args.results_dir else []
        except ET.ParseError:
            failures = []
    failures = sorted(set(filter(None, failures)))
    # 不知道是哪个用例失败时也要报，不然构建挂了一直没人知道。
    if not failures:
        failures = ["作业"]

    open_issues = json.loads(
        gh(
            "issue", "list", "--repo", args.repo, "--state", "open", "--limit", "100",
            "--search", f'"{TITLE_PREFIX}" in:title', "--json", "number,title",
        )
    )
    # 之前开出来的标题带 xcresult 的 "()" 后缀，按去掉后的比。
    by_title = {issue["title"].removesuffix("()"): issue["number"] for issue in open_issues}

    workflow = os.environ.get("GITHUB_WORKFLOW", "nightly-ui")
    platform_name = "iOS" if args.platform == "ios" else "Android"
    failed = 0
    for test in failures:
        title = f"{TITLE_PREFIX}{platform_name} {test}"
        body = (
            f"{args.run_url} 失败。\n\n"
            f"端：{platform_name}\n测试：{test}\n\n"
            f"`{workflow}` 夜跑自动开的 issue；同一条测试再失败就在这条上留言。查清原因后关掉。"
        )
        try:
            if title.removesuffix("()") in by_title:
                gh("issue", "comment", str(by_title[title.removesuffix("()")]), "--repo", args.repo, "--body", args.run_url)
                print(f"留言：{title}")
            else:
                gh("issue", "create", "--repo", args.repo, "--title", title, "--body", body)
                print(f"新开：{title}")
        except subprocess.CalledProcessError as error:
            failed += 1
            print(f"上报失败：{title}\n{error.stderr}", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
