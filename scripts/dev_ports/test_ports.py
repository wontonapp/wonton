"""端口检查逻辑的单元测试：短写法、长写法、变量默认值、例外和整个仓库的扫描。

运行：python3 -m unittest discover -s scripts/dev_ports -t scripts（check.sh 和 CI 都跑）
"""

from __future__ import annotations

import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from dev_ports.ports import check_entry, check_file, check_repo, loopback_default, short_segments


class LoopbackDefault(unittest.TestCase):
    def test_literals(self):
        self.assertTrue(loopback_default("127.0.0.1"))
        self.assertTrue(loopback_default("::1"))
        self.assertTrue(loopback_default("[::1]"))

    def test_rejects(self):
        for t in ["0.0.0.0", "localhost", "", "${V}", "${V:-0.0.0.0}", "127.0.0.2"]:
            self.assertFalse(loopback_default(t), t)

    def test_variable_with_loopback_default(self):
        self.assertTrue(loopback_default("${WONTON_DEV_TUNNEL_BIND:-127.0.0.1}"))
        self.assertTrue(loopback_default("${V:-::1}"))


class ShortSegments(unittest.TestCase):
    def test_keeps_variable_defaults_whole(self):
        self.assertEqual(
            short_segments("${A:-127.0.0.1}:${B:-80}:80"),
            ["${A:-127.0.0.1}", "${B:-80}", "80"],
        )

    def test_keeps_ipv6_bracket_whole(self):
        self.assertEqual(short_segments("[::1]:8080:80"), ["[::1]", "8080", "80"])

    def test_plain(self):
        self.assertEqual(short_segments("127.0.0.1:8080-8090:80-90/udp"), ["127.0.0.1", "8080-8090", "80-90/udp"])
        self.assertEqual(short_segments("8080:80"), ["8080", "80"])


class CheckEntry(unittest.TestCase):
    def test_short_ok(self):
        for e in [
            "127.0.0.1:8080:80",
            "[::1]:8080:80",
            "127.0.0.1:8080-8090:80-90/udp",
            "${V:-127.0.0.1}:8080:80",
            "${V:-::1}:8080:80",
        ]:
            self.assertIsNone(check_entry(e), e)

    def test_short_bad(self):
        for e, why in [
            ("8080:80", "只写了端口"),
            ("8080", "只写了端口"),
            ("0.0.0.0:8080:80", "宿主地址"),
            ("${V}:8080:80", "宿主地址"),
            ("${V:-0.0.0.0}:8080:80", "宿主地址"),
            (8080, "不是字符串写法"),
        ]:
            reason = check_entry(e)
            self.assertIsNotNone(reason, e)
            self.assertIn(why, reason)

    def test_long_ok(self):
        self.assertIsNone(check_entry({"target": 80, "published": 8080, "host_ip": "127.0.0.1"}))
        self.assertIsNone(check_entry({"target": 80, "published": "8080", "host_ip": "${V:-127.0.0.1}", "protocol": "udp"}))

    def test_long_bad(self):
        self.assertIn("缺 host_ip", check_entry({"target": 80, "published": 8080}))
        self.assertIn("host_ip", check_entry({"target": 80, "published": 8080, "host_ip": "0.0.0.0"}))
        self.assertIn("host_ip", check_entry({"target": 80, "published": 8080, "host_ip": "${V}"}))


class CheckFile(unittest.TestCase):
    def write_compose(self, root: str, rel: str, text: str) -> str:
        path = os.path.join(root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        return rel

    def test_flags_service_and_respects_exception(self):
        with tempfile.TemporaryDirectory() as root:
            rel = self.write_compose(
                root,
                "deploy/dev/x/compose.yaml",
                """
services:
  open:
    ports: ["8080:80"]
  bound:
    ports: ["127.0.0.1:8080:80"]
""",
            )
            violations = check_file(root, rel, {})
            self.assertEqual([(v.service, v.entry) for v in violations], [("open", "'8080:80'")])
            # 例外按文件 + 服务收窄：同文件的其他服务照样报。
            scoped = check_file(root, rel, {(rel, "open"): "原因"})
            self.assertEqual(scoped, [])
            other_file = self.write_compose(
                root,
                "deploy/dev/y/compose.yaml",
                """
services:
  open:
    ports: ["8080:80"]
""",
            )
            # 例外不跨文件：同名服务在别的文件里照报。
            self.assertEqual(len(check_file(root, other_file, {(rel, "open"): "原因"})), 1)

    def test_broken_yaml_is_one_violation(self):
        with tempfile.TemporaryDirectory() as root:
            rel = self.write_compose(root, "deploy/dev/bad/compose.yaml", "services: [oops\n")
            violations = check_file(root, rel, {})
            self.assertEqual(len(violations), 1)
            self.assertEqual(violations[0].service, "(整个文件)")

    def test_long_syntax_in_file(self):
        with tempfile.TemporaryDirectory() as root:
            rel = self.write_compose(
                root,
                "deploy/dev/long/compose.yaml",
                """
services:
  a:
    ports:
      - target: 80
        published: 8080
""",
            )
            violations = check_file(root, rel, {})
            self.assertEqual([v.reason for v in violations], ["长写法缺 host_ip，等于绑 0.0.0.0"])


class CheckRepo(unittest.TestCase):
    def test_walks_experiments_and_ignores_other_dirs(self):
        with tempfile.TemporaryDirectory() as root:
            os.makedirs(os.path.join(root, "deploy", "prod"))
            for rel, text in [
                ("deploy/dev/ok/compose.yaml", "services:\n  a:\n    ports: ['127.0.0.1:1:1']\n"),
                ("deploy/dev/experiments/deep/docker-compose.yml", "services:\n  a:\n    ports: ['2:2']\n"),
                ("deploy/prod/naughty/compose.yaml", "services:\n  a:\n    ports: ['3:3']\n"),
            ]:
                path = os.path.join(root, rel)
                os.makedirs(os.path.dirname(path), exist_ok=True)
                with open(path, "w", encoding="utf-8") as f:
                    f.write(text)
            violations = check_repo(root, {})
            self.assertEqual([(v.file, v.service) for v in violations], [("deploy/dev/experiments/deep/docker-compose.yml", "a")])


if __name__ == "__main__":
    unittest.main()
