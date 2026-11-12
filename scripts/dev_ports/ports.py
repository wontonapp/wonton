"""deploy/dev/ 下 compose 文件的端口门禁：宿主地址只许绑 127.0.0.1（或 [::1]）。

起因（2026-11-08 #824）：开发栈的端口被从 127.0.0.1 改成听所有网卡提交进了 PR，审查时才发现。
开发者自己设变量放开是有意的（比如 WONTON_DEV_TUNNEL_BIND=0.0.0.0），但文件里的默认值必须是回环。
按 YAML 解析，不按行正则硬抓；短写法、长写法、变量默认值都要认。
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass
from typing import Dict, Iterator, List, Optional, Tuple

try:
    import yaml
except ImportError as e:  # pragma: no cover - 环境缺依赖时的报错指引
    raise ImportError("需要 PyYAML：python3 -m pip install --user pyyaml（或发行版包 python-yaml）") from e

COMPOSE_NAMES = ("compose.yaml", "compose.yml", "docker-compose.yaml", "docker-compose.yml")

# 宿主地址合格的取值：回环字面量，或默认值是回环的变量。
LOOPBACK = ("127.0.0.1", "::1")

VAR_RE = re.compile(r"^\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}$")

# 有意的例外：文件 + 服务 → 原因。收得很窄：这个服务的这个行为是设计如此；设计改了就删掉。
# 例外不进白名单文件而是写在代码里，因为端口放开必须能被一眼看到为什么。
EXCEPTIONS: Dict[Tuple[str, str], str] = {
    ("deploy/dev/agent/compose.yaml", "tuwunel"): "局域网里的手机客户端直连测试，监听所有网卡是有意的（compose.yaml 注释同此）",
}


@dataclass
class Violation:
    file: str
    service: str
    entry: str
    reason: str

    def describe(self) -> str:
        return f"{self.file} 服务 {self.service}：{self.entry} —— {self.reason}"


def find_compose_files(root: str) -> Iterator[str]:
    """deploy/dev/ 下（含 experiments）所有 compose 文件，相对 root 的路径，排序保证输出稳定。"""
    dev = os.path.join(root, "deploy", "dev")
    if not os.path.isdir(dev):
        return
    for dirpath, dirnames, filenames in os.walk(dev):
        dirnames.sort()
        for name in sorted(filenames):
            if name in COMPOSE_NAMES:
                yield os.path.relpath(os.path.join(dirpath, name), root).replace(os.sep, "/")


def loopback_default(token: str) -> bool:
    """回环字面量，或默认值是回环的变量（${VAR:-127.0.0.1}）。${VAR} 不带默认值不算。"""
    t = token.strip()
    if t.startswith("[") and t.endswith("]"):
        t = t[1:-1]
    if t in LOOPBACK:
        return True
    m = VAR_RE.match(t)
    if m and m.group(2) is not None:
        d = m.group(2).strip()
        if d.startswith("[") and d.endswith("]"):
            d = d[1:-1]
        return d in LOOPBACK
    return False


def short_segments(entry: str) -> List[str]:
    """短写法按冒号切段，跳过 ${...} 和段首 [...] 里的冒号（IPv6、变量默认值都可能是 127.0.0.1:xx 这种形状）。"""
    parts: List[str] = []
    cur: List[str] = []
    var_depth = 0
    bracket_depth = 0
    i = 0
    n = len(entry)
    while i < n:
        if entry.startswith("${", i):
            var_depth += 1
            cur.append("${")
            i += 2
            continue
        ch = entry[i]
        if var_depth > 0:
            if ch == "}":
                var_depth -= 1
            cur.append(ch)
            i += 1
            continue
        if ch == "[" and not cur:
            bracket_depth = 1
            cur.append(ch)
            i += 1
            continue
        if bracket_depth > 0:
            if ch == "]":
                bracket_depth -= 1
            cur.append(ch)
            i += 1
            continue
        if ch == ":":
            parts.append("".join(cur))
            cur = []
            i += 1
            continue
        cur.append(ch)
        i += 1
    parts.append("".join(cur))
    return parts


def check_entry(entry: object) -> Optional[str]:
    """一条 ports 项：合格返回 None，不合格返回原因。"""
    if isinstance(entry, dict):
        # 长写法：host_ip 缺省等于绑 0.0.0.0，也按不合格报。
        host_ip = entry.get("host_ip")
        if host_ip is None:
            return "长写法缺 host_ip，等于绑 0.0.0.0"
        if isinstance(host_ip, str) and loopback_default(host_ip):
            return None
        return f"长写法 host_ip {host_ip!r} 不是 127.0.0.1 / [::1]"
    if not isinstance(entry, str):
        # YAML 可能把它解析成别的标量（没加引号的 "8080:10" 这类），原样报出来人工看。
        return f"不是字符串写法：{entry!r}"
    text = entry.strip()
    if not text:
        return "空条目"
    parts = short_segments(text)
    host = parts[0] if parts else ""
    if not loopback_default(host):
        if len(parts) >= 3:
            return f"宿主地址 {host} 不是 127.0.0.1 / [::1]"
        return "只写了端口没写宿主地址，等于绑 0.0.0.0"
    return None


def entry_text(entry: object) -> str:
    """把那一项按它写出来的样子展示。"""
    if isinstance(entry, str):
        return repr(entry)
    if isinstance(entry, dict):
        inner = ", ".join(f"{k}={v!r}" for k, v in sorted(entry.items()))
        return "{" + inner + "}"
    return repr(entry)


def check_file(root: str, rel: str, exceptions: Dict[Tuple[str, str], str]) -> List[Violation]:
    """一个 compose 文件里所有不合格的 ports 项；YAML 坏了整个文件报一条。"""
    with open(os.path.join(root, rel), encoding="utf-8") as f:
        try:
            doc = yaml.safe_load(f)
        except yaml.YAMLError as e:
            return [Violation(rel, "(整个文件)", rel, f"YAML 解析失败：{e}")]
    if not isinstance(doc, dict):
        return []
    services = doc.get("services")
    if not isinstance(services, dict):
        return []
    out: List[Violation] = []
    for name in sorted(services):
        service = services[name]
        if not isinstance(service, dict):
            continue
        ports = service.get("ports")
        if ports is None:
            continue
        if not isinstance(ports, list):
            if (rel, name) not in exceptions:
                out.append(Violation(rel, name, entry_text(ports), "ports 不是列表"))
            continue
        for entry in ports:
            reason = check_entry(entry)
            if reason is None or (rel, name) in exceptions:
                continue
            out.append(Violation(rel, name, entry_text(entry), reason))
    return out


def check_repo(root: str, exceptions: Dict[Tuple[str, str], str]) -> List[Violation]:
    """扫 deploy/dev/ 下全部 compose 文件，汇总不合格项。"""
    problems: List[Violation] = []
    for rel in find_compose_files(root):
        problems.extend(check_file(root, rel, exceptions))
    return problems
