#!/usr/bin/env python3
"""冒烟用的索引变体：update / missing / future。打印新的 docs.commit。

- update：在固定的 devices 页后面加一段冒烟标记，commit 换成不会撞上的 40 位十六进制，
  并把原页的 sha256 记到 original-devices.sha；
- missing：user/zh/devices.md 换成一个 blobs 里不存在的 sha256，核心取文件失败（本地索引按 sha256 取文件，
  失败路径和断网取不到文件是同一条）。
- future：devices 换回没打标记的原页、commit 也换新的，min_core 比核心新。如果核心
  误把 min_core 当空气，docs_commit 会变、正文里的标记会消失。
"""
import hashlib
import json
import sys
from pathlib import Path

store = Path(sys.argv[1])
index = json.loads((store / "index.json").read_text())
mode = sys.argv[2]
devices = "user/zh/devices.md"

if mode == "update":
    blobs = store / "blobs"
    page = (blobs / index["docs"]["files"][devices]).read_text()
    updated = page.rstrip() + "\n\n## 冒烟标记\n\n这是引导冒烟场景 3 的更新内容。\n"
    sha = hashlib.sha256(updated.encode()).hexdigest()
    (blobs / sha).write_text(updated)
    (store / "original-devices.sha").write_text(index["docs"]["files"][devices])
    index["docs"]["files"][devices] = sha
elif mode == "missing":
    # 本地索引按 sha256 取文件：指向一个不存在的 blob，核心取文件失败，旧文档照常用。
    index["docs"]["files"][devices] = hashlib.sha256(b"onboarding-smoke-missing-blob").hexdigest()
elif mode == "future":
    original = (store / "original-devices.sha").read_text().strip()
    index["docs"]["files"][devices] = original
    index["docs"]["min_core"] = "99.0.0"
else:
    sys.exit(f"unknown mode {mode}")

index["docs"]["commit"] = hashlib.sha256(f"onboarding-smoke-docs-{mode}".encode()).hexdigest()[:40]
(store / "index.json").write_text(json.dumps(index, ensure_ascii=False, indent=1))
print(index["docs"]["commit"])
