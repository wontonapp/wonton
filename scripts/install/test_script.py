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
        error = os.environ.get("FAKE_DOCKER_PULL_ERROR")
        if error:
            print(error, file=sys.stderr)
            sys.exit(1)
        out("pulled " + args[-1])
    elif command == "image":
        fmt = value("--format")
        reference = args[-1]
        if "RepoDigests" in fmt:
            for digest in digests.get(reference, []):
                out(digest)
        elif reference not in digests:
            sys.exit(1)
    elif command == "run":
        env_file = value("--env-file")
        entries = {}
        if env_file:
            with open(env_file, encoding="utf-8") as file:
                for line in file:
                    key, _, item = line.rstrip("\n").partition("=")
                    entries[key] = item
        if "init" in args:
            # init 要读复制进来的文件；读不到就是安装失败。
            for key, item in entries.items():
                if key.endswith("_FILE") and not os.path.isfile(item):
                    out(f"missing input {key}={item}")
                    sys.exit(8)
            if not entries.get("WONTON_CLIENT_SERVICE"):
                sys.exit(7)
            install = args[args.index("init") + 1]
            os.makedirs(os.path.join(install, "entry"), exist_ok=True)
            with open(os.path.join(install, "entry", "settings.json"), "w", encoding="utf-8") as file:
                json.dump({}, file)
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
        self.invite = self.temp / "invite"
        self.invite.write_text("invite\n")
        self.ca = self.temp / "ca.pem"
        self.ca.write_text("ca\n")
        self.digests = {}

    def write_fake(self, name, body):
        path = self.bin / name
        path.write_text(body)
        path.chmod(0o755)

    def run_install(self, entry=True, **extra):
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
        if entry:
            env.update({
                "WONTON_CLIENT_SERVICE": "https://names.test",
                "WONTON_CLIENT_INVITE_FILE": str(self.invite),
                "WONTON_CLIENT_CA_FILE": str(self.ca),
                "WONTON_FRP_CA_FILE": str(self.ca),
                "WONTON_ACME_DIRECTORY": "https://pebble.test/dir",
                "WONTON_ACME_ACCEPT_TERMS": "true",
            })
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
        self.digests["ghcr.io/wontonapp/wonton-core:stable"] = [
            "ghcr.io/wontonapp/wonton-core@" + digest
        ]

        result = self.run_install()

        self.assertEqual(result.returncode, 0, result.stderr)
        env = self.env()
        self.assertEqual(env["WONTON_CORE_IMAGE"], "ghcr.io/wontonapp/wonton-core@" + digest)
        # 渠道照旧记标签：核心从 WONTON_CORE_SOURCE 读渠道。
        self.assertEqual(env["WONTON_CORE_SOURCE"], "ghcr.io/wontonapp/wonton-core:stable")
        self.assertEqual(self.calls("pull")[-1], ["pull", "ghcr.io/wontonapp/wonton-core:stable"])
        for run in self.runs():
            self.assertIn("ghcr.io/wontonapp/wonton-core@" + digest, run)
            self.assertNotIn("ghcr.io/wontonapp/wonton-core:stable", run)

    def test_a_registry_port_is_kept_in_the_repository(self):
        reference = "192.0.2.10:5000/wonton/core:stable"
        digest = "sha256:" + "b" * 64
        self.digests[reference] = ["192.0.2.10:5000/wonton/core@" + digest]

        result = self.run_install(WONTON_CORE_IMAGE=reference)

        self.assertEqual(result.returncode, 0, result.stderr)
        env = self.env()
        self.assertEqual(env["WONTON_CORE_IMAGE"], "192.0.2.10:5000/wonton/core@" + digest)
        self.assertEqual(env["WONTON_CORE_SOURCE"], reference)

    def test_missing_given_name_configuration_stops_at_init(self):
        self.digests["ghcr.io/geraldpeng6/wonton-core:beta"] = [
            "ghcr.io/geraldpeng6/wonton-core@sha256:" + "e" * 64
        ]

        result = self.run_install(entry=False, WONTON_CORE_IMAGE="ghcr.io/geraldpeng6/wonton-core:beta")

        self.assertNotEqual(result.returncode, 0)
        log = (self.state / "log").read_text()
        self.assertIn("init", log)
        self.assertFalse((self.install_dir / ".env").exists())

    def test_rerun_keeps_recorded_entry_inputs(self):
        self.digests["ghcr.io/wontonapp/wonton-core:stable"] = [
            "ghcr.io/wontonapp/wonton-core@sha256:" + "f" * 64
        ]
        result = self.run_install()
        self.assertEqual(result.returncode, 0, result.stderr)
        inputs = self.install_dir / "entry-input"

        # 升级常是直接再跑一次安装，不给入口变量：记录和副本都还在，续期才读得到。
        result = self.run_install(entry=False)

        self.assertEqual(result.returncode, 0, result.stderr)
        recorded = (inputs / "init.env").read_text()
        for key in [
            "WONTON_CLIENT_SERVICE",
            "WONTON_CLIENT_INVITE_FILE",
            "WONTON_CLIENT_CA_FILE",
            "WONTON_FRP_CA_FILE",
        ]:
            self.assertIn(key + "=", recorded)
        for line in recorded.splitlines():
            key, _, value = line.partition("=")
            if key.endswith("_FILE"):
                self.assertTrue(Path(value).is_file(), f"{key} 的副本被清掉了")

    def test_no_matching_repository_digest_stops_instead_of_writing_the_tag(self):
        reference = "ghcr.io/wontonapp/wonton-core:stable"
        self.digests[reference] = ["ghcr.io/elsewhere/core@sha256:" + "c" * 64]

        result = self.run_install()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("没有仓库 ghcr.io/wontonapp/wonton-core 的 digest", result.stderr)
        self.assertFalse((self.install_dir / ".env").exists())

    def test_an_explicit_digest_is_kept_and_leaves_the_channel(self):
        reference = "ghcr.io/wontonapp/wonton-core@sha256:" + "d" * 64

        result = self.run_install(WONTON_CORE_IMAGE=reference)

        self.assertEqual(result.returncode, 0, result.stderr)
        env = self.env()
        self.assertEqual(env["WONTON_CORE_IMAGE"], reference)
        self.assertEqual(env["WONTON_CORE_SOURCE"], reference)

    def test_install_timeout_is_written_when_set_and_kept_across_reruns(self):
        digest = "sha256:" + "e" * 64
        self.digests["ghcr.io/wontonapp/wonton-core:stable"] = [
            "ghcr.io/wontonapp/wonton-core@" + digest
        ]

        # 没设时不写这一行，核心用自己的默认值。
        result = self.run_install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("WONTON_INSTALL_TIMEOUT_SECS", self.env())

        # 设了写进 .env，compose 的白名单把它传进核心容器。
        result = self.run_install(WONTON_INSTALL_TIMEOUT_SECS="600")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.env()["WONTON_INSTALL_TIMEOUT_SECS"], "600")

        # 重跑（升级、修复）重写 .env 时沿用保存的值，不用再给一遍。
        result = self.run_install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.env()["WONTON_INSTALL_TIMEOUT_SECS"], "600")

        # 设成空就是取消：回到核心默认，回写时不再有这一行。
        result = self.run_install(WONTON_INSTALL_TIMEOUT_SECS="")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("WONTON_INSTALL_TIMEOUT_SECS", self.env())

    def pull_failure(self, error, **extra):
        result = self.run_install(FAKE_DOCKER_PULL_ERROR=error, **extra)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.install_dir / ".env").exists())
        return result

    def test_a_missing_tag_says_the_channel_has_no_release(self):
        result = self.pull_failure("Error response from daemon: manifest unknown")

        self.assertIn("渠道 stable 还没有发布的版本", result.stderr)
        self.assertIn("WONTON_CORE_IMAGE", result.stderr)
        self.assertNotIn("配代理", result.stderr)
        # 人话在前，Docker 原文收在“详情”后面。
        self.assertLess(result.stderr.index("详情："), result.stderr.index("manifest unknown"))

        # 给的是 digest 时没有渠道可说。
        digest = "sha256:" + "f" * 64
        result = self.pull_failure(
            "Error response from daemon: manifest unknown",
            WONTON_CORE_IMAGE="ghcr.io/wontonapp/wonton-core@" + digest,
        )
        self.assertNotIn("渠道", result.stderr.splitlines()[0])

    def test_a_denied_pull_says_the_image_may_not_be_public(self):
        result = self.pull_failure(
            'Error response from daemon: Head "https://ghcr.io/v2/wontonapp/wonton-core/manifests/stable": unauthorized'
        )

        self.assertIn("可能还没公开", result.stderr)
        self.assertIn("WONTON_CORE_IMAGE", result.stderr)
        self.assertNotIn("配代理", result.stderr)
        self.assertLess(result.stderr.index("详情："), result.stderr.index("unauthorized"))

    def test_a_registry_outage_keeps_the_proxy_help(self):
        result = self.pull_failure(
            'Error response from daemon: Get "https://ghcr.io/v2/": dial tcp 192.0.2.10:443: i/o timeout'
        )

        self.assertIn("连不上 Docker 镜像仓库", result.stderr)
        self.assertIn("proxy.conf", result.stderr)

    def test_an_unknown_pull_error_shows_the_docker_output_as_details(self):
        result = self.pull_failure("Error response from daemon: unexpected EOF")

        self.assertIn("拉不到镜像", result.stderr)
        self.assertLess(result.stderr.index("详情："), result.stderr.index("unexpected EOF"))


if __name__ == "__main__":
    unittest.main()
