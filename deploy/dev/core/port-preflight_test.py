#!/usr/bin/env python3
"""port-preflight.sh（dev.sh up 构建前的端口检查）的行为测试：假 docker ps、假 ss，不连 Docker。

假 docker 按 FAKE_PS 回答 `docker ps`（每行“项目|容器名|端口”），假 ss 按 FAKE_LISTEN（“<端口>/<tcp|udp>”的列表）
回答本机有没有 socket。也测给 ACME 管理口分口的 free / next-free 两个模式。
"""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).parent
SCRIPT = HERE / "port-preflight.sh"

FAKE_DOCKER = """#!/usr/bin/env bash
[[ $1 == ps ]] && printf '%b' "${FAKE_PS:-}"
exit 0
"""
FAKE_SS = """#!/usr/bin/env bash
proto=tcp; [[ $1 == *u* ]] && proto=udp
p=${2##*:}
for x in ${FAKE_LISTEN:-}; do [[ $x == "$p/$proto" ]] && echo "UNCONN 0 0 0.0.0.0:$p 0.0.0.0:*"; done
exit 0
"""


class PortPreflight(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        bin_dir = Path(self.tmp.name)
        for name, body in (("docker", FAKE_DOCKER), ("ss", FAKE_SS)):
            path = bin_dir / name
            path.write_text(body)
            path.chmod(0o755)
        self.path = f"{bin_dir}:{os.environ['PATH']}"

    def run_check(self, ps="", listen="", docker_host=None, acme="18484", ports=None):
        env = dict(os.environ, PATH=self.path, FAKE_PS=ps, FAKE_LISTEN=listen)
        env.pop("DOCKER_HOST", None)
        if docker_host:
            env["DOCKER_HOST"] = docker_host
        args = ports if ports is not None else ["wonton-mine", "18480", "18482", acme]
        result = subprocess.run(
            ["bash", str(SCRIPT), *args],
            env=env, capture_output=True, text=True, timeout=30,
        )
        return result.returncode, result.stderr

    def run_mode(self, *args, ps="", listen=""):
        env = dict(os.environ, PATH=self.path, FAKE_PS=ps, FAKE_LISTEN=listen)
        env.pop("DOCKER_HOST", None)
        result = subprocess.run(
            ["bash", str(SCRIPT), *args],
            env=env, capture_output=True, text=True, timeout=30,
        )
        return result.returncode, result.stdout, result.stderr

    def test_free_ports_pass(self):
        self.assertEqual(self.run_check(), (0, ""))

    def test_other_stack_container_on_tunnel_port(self):
        rc, err = self.run_check(
            ps="wonton-other|wonton-other|127.0.0.1:18680->18680/tcp, 0.0.0.0:18482->18482/udp\\n"
        )
        self.assertEqual(rc, 1)
        self.assertIn("18482/udp", err)
        self.assertIn("wonton-other", err)
        self.assertIn("WONTON_DEV_TUNNEL_PORT=18483", err)
        self.assertNotIn("18480/tcp", err)

    def test_host_socket_on_gateway_port(self):
        rc, err = self.run_check(listen="18480/tcp 18481/tcp")
        self.assertEqual(rc, 1)
        self.assertIn("18480/tcp", err)
        self.assertIn("WONTON_DEV_PORT=18482", err)

    def test_own_stack_ports_are_free(self):
        ps = "wonton-mine|wonton-mine|127.0.0.1:18480->18480/tcp, 0.0.0.0:18482->18482/udp\\n"
        self.assertEqual(self.run_check(ps=ps, listen="18480/tcp 18482/udp"), (0, ""))

    def test_acme_port_conflict(self):
        rc, err = self.run_check(
            ps="wonton-other|wonton-other|127.0.0.1:18484->8055/tcp\\n"
        )
        self.assertEqual(rc, 1)
        self.assertIn("18484/tcp", err)
        self.assertIn("WONTON_DEV_ACME_DNS_PORT=18485", err)

    def test_free_reports_free_or_holder(self):
        self.assertEqual(self.run_mode("free", "wonton-mine", "18484", "tcp"), (0, "", ""))
        rc, out, _ = self.run_mode("free", "wonton-mine", "18484", "tcp", listen="18484/tcp")
        self.assertEqual(rc, 1)
        self.assertIn("18484", out)

    def test_free_counts_own_container_as_free(self):
        ps = "wonton-mine|wonton-mine|127.0.0.1:18484->8055/tcp\\n"
        self.assertEqual(self.run_mode("free", "wonton-mine", "18484", "tcp", ps=ps), (0, "", ""))

    def test_next_free_skips_taken_ports(self):
        # 从 18484 起找：自己的容器占着的算空，别的项目和本机 socket 跳过。
        ps = ("wonton-other|wonton-other|127.0.0.1:18484->8055/tcp, 127.0.0.1:18486->8055/tcp\\n"
              "wonton-mine|wonton-mine|127.0.0.1:18485->8055/tcp\\n")
        rc, out, _ = self.run_mode("next-free", "wonton-mine", "18484", "8", ps=ps, listen="18487/tcp")
        self.assertEqual(rc, 0)
        self.assertEqual(out.strip(), "18485")

    def test_next_free_all_taken(self):
        rc, out, err = self.run_mode("next-free", "wonton-mine", "18484", "2", listen="18484/tcp 18485/tcp")
        self.assertEqual(rc, 3)
        self.assertEqual(out, "")
        self.assertIn("18484–18485/tcp", err)

    def test_protocols_are_separate(self):
        self.assertEqual(self.run_check(listen="18482/tcp 18480/udp"), (0, ""))

    def test_remote_docker_ignores_local_sockets(self):
        self.assertEqual(self.run_check(listen="18480/tcp", docker_host="ssh://elsewhere"), (0, ""))
        rc, _ = self.run_check(
            ps="x|wonton-x|0.0.0.0:18482->18482/udp\\n", docker_host="ssh://elsewhere"
        )
        self.assertEqual(rc, 1)


if __name__ == "__main__":
    unittest.main()
