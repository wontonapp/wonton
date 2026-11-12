"""安装脚本写进 .env 的核心镜像要是仓库 digest，渠道照旧留在 WONTON_CORE_SOURCE。

标签会跟着渠道移走：装着标签时，第一次升级失败后的回滚会起到新版本，却照样报回滚成功（#669）。
假的 docker 只回答脚本要问的几件事；能跑在 CI 上，不需要 Docker。
"""

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[2] / "deploy" / "install" / "install.sh"

FAKE_DOCKER = r'''#!/usr/bin/env python3
import json, os, sys

state = os.environ["FAKE_DOCKER_STATE"]
args = sys.argv[1:]
with open(os.path.join(state, "log"), "a", encoding="utf-8") as log:
    log.write(json.dumps(args) + "\n")
with open(os.path.join(state, "digests.json"), encoding="utf-8") as f:
    digests = json.load(f)


def out(text=""):
    print(text)


def value(flag):
    return args[args.index(flag) + 1] if flag in args else ""


def main():
    command = args[0] if args else ""
    if command == "version":
        out("29.7.2")
    elif command == "info":
        fmt = value("--format")
        if "DockerRootDir" in fmt:
            out(state)
        elif "CgroupDriver" in fmt:
            out("cgroupfs 1")
        elif "Proxy" in fmt:
            out("\n")
    elif command == "pull":
        out("pulled " + args[-1])
    elif command == "image":
        fmt = value("--format")
        reference = args[-1]
        if "RepoDigests" in fmt:
            for digest in digests.get(reference, []):
                out(digest)
        elif reference not in digests:
            sys.exit(1)
    elif command == "inspect":
        # 没有 updater 容器，也没有别的容器要查。
        sys.exit(1)
    return 0


sys.exit(main())
'''


class InstallTest(unittest.TestCase):
    def setUp(self):
        self.temp = Path(tempfile.mkdtemp(prefix="p669-install-"))
        self.addCleanup(shutil.rmtree, self.temp)
        self.state = self.temp / "state"
        self.state.mkdir()
        self.bin = self.temp / "bin"
        self.bin.mkdir()
        self.install_dir = self.temp / "opt"
        self.write_fake("docker", FAKE_DOCKER)
        self.write_fake("id", "#!/bin/sh\necho 0\n")
        self.write_fake("uname", "#!/bin/sh\necho x86_64\n")
        self.write_fake("ln", "#!/bin/sh\nexit 0\n")
        self.write_fake(
            "mkdir",
            "#!/bin/sh\nfor a in \"$@\"; do last=$a; done\n"
            "case \"$last\" in /usr/local/*) exit 0 ;; esac\nexec /bin/mkdir \"$@\"\n",
        )
        (self.temp / "docker-config").mkdir()
        self.digests = {}

    def write_fake(self, name, body):
        path = self.bin / name
        path.write_text(body)
        path.chmod(0o755)

    def run_install(self, **extra):
        (self.state / "digests.json").write_text(json.dumps(self.digests))
        (self.state / "log").write_text("")
        env = {
            "PATH": f"{self.bin}:{os.environ['PATH']}",
            "HOME": str(self.temp),
            "DOCKER_CONFIG": str(self.temp / "docker-config"),
            "TMPDIR": str(self.temp),
            "TZ": "UTC",
            "WONTON_INSTALL_DIR": str(self.install_dir),
            "WONTON_DATA_DIR": str(self.temp / "data"),
            "FAKE_DOCKER_STATE": str(self.state),
            **extra,
        }
        return subprocess.run(
            ["sh", str(SCRIPT)],
            env=env,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=120,
        )

    def env(self):
        return dict(
            line.split("=", 1)
            for line in (self.install_dir / ".env").read_text().splitlines()
            if "=" in line and not line.startswith("#")
        )

    def calls(self, command):
        log = (self.state / "log").read_text().splitlines()
        return [json.loads(line) for line in log if json.loads(line)[0] == command]

    def runs(self):
        return self.calls("run")

    def test_a_channel_tag_is_resolved_to_its_repository_digest(self):
        digest = "sha256:" + "a" * 64
        self.digests["ghcr.io/geraldpeng6/wonton-core:beta"] = [
            "ghcr.io/geraldpeng6/wonton-core@" + digest
        ]

        result = self.run_install()

        self.assertEqual(result.returncode, 0, result.stderr)
        env = self.env()
        self.assertEqual(env["WONTON_CORE_IMAGE"], "ghcr.io/geraldpeng6/wonton-core@" + digest)
        # 渠道照旧记标签：核心从 WONTON_CORE_SOURCE 读渠道。
        self.assertEqual(env["WONTON_CORE_SOURCE"], "ghcr.io/geraldpeng6/wonton-core:beta")
        self.assertEqual(self.calls("pull")[-1], ["pull", "ghcr.io/geraldpeng6/wonton-core:beta"])
        for run in self.runs():
            self.assertIn("ghcr.io/geraldpeng6/wonton-core@" + digest, run)
            self.assertNotIn("ghcr.io/geraldpeng6/wonton-core:beta", run)

    def test_a_registry_port_is_kept_in_the_repository(self):
        reference = "192.0.2.10:5000/wonton/core:stable"
        digest = "sha256:" + "b" * 64
        self.digests[reference] = ["192.0.2.10:5000/wonton/core@" + digest]

        result = self.run_install(WONTON_CORE_IMAGE=reference)

        self.assertEqual(result.returncode, 0, result.stderr)
        env = self.env()
        self.assertEqual(env["WONTON_CORE_IMAGE"], "192.0.2.10:5000/wonton/core@" + digest)
        self.assertEqual(env["WONTON_CORE_SOURCE"], reference)

    def test_no_matching_repository_digest_stops_instead_of_writing_the_tag(self):
        reference = "ghcr.io/geraldpeng6/wonton-core:beta"
        self.digests[reference] = ["ghcr.io/elsewhere/core@sha256:" + "c" * 64]

        result = self.run_install()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("没有仓库 ghcr.io/geraldpeng6/wonton-core 的 digest", result.stderr)
        self.assertFalse((self.install_dir / ".env").exists())

    def test_an_explicit_digest_is_kept_and_leaves_the_channel(self):
        reference = "ghcr.io/geraldpeng6/wonton-core@sha256:" + "d" * 64

        result = self.run_install(WONTON_CORE_IMAGE=reference)

        self.assertEqual(result.returncode, 0, result.stderr)
        env = self.env()
        self.assertEqual(env["WONTON_CORE_IMAGE"], reference)
        self.assertEqual(env["WONTON_CORE_SOURCE"], reference)


if __name__ == "__main__":
    unittest.main()
