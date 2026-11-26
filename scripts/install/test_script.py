"""安装脚本写进 .env 的核心镜像要是仓库 digest，渠道照旧留在 WONTON_CORE_SOURCE。

标签会跟着渠道移走：装着标签时，第一次升级失败后的回滚会起到新版本，却照样报回滚成功（#669）。
假的 docker 只回答脚本要问的几件事；能跑在 CI 上，不需要 Docker。
"""

import errno
import json
import os
import pty
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
            if not entries.get("WONTON_CLIENT_SERVICE") and entries.get("WONTON_MATRIX_NAME_MODE") != "none":
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

    def write_pairing_command(self):
        code = "test-only-one-time-pairing-code"
        self.install_dir.mkdir()
        path = self.install_dir / "wonton"
        path.write_text(f"#!/bin/sh\nprintf '%s\\n' '{code}'\n")
        path.chmod(0o755)
        return code

    def write_fake(self, name, body):
        path = self.bin / name
        path.write_text(body)
        path.chmod(0o755)

    def run_install(self, entry=True, terminal=False, **extra):
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
            # 测试不以 root 跑：中继目录的默认值 `/run/wonton/relay` 建不出来。
            "WONTON_RELAY_DIR": str(self.temp / "relay"),
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
        master, slave = pty.openpty() if terminal else (None, None)
        try:
            result = subprocess.run(
                ["sh", str(SCRIPT)],
                env=env,
                stdin=subprocess.DEVNULL,
                stdout=slave if terminal else subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                timeout=120,
            )
            if terminal:
                os.close(slave)
                slave = None
                output = bytearray()
                while True:
                    try:
                        chunk = os.read(master, 4096)
                    except OSError as error:
                        if error.errno != errno.EIO:
                            raise
                        break
                    if not chunk:
                        break
                    output.extend(chunk)
                result.stdout = output.decode()
            return result
        finally:
            if slave is not None:
                os.close(slave)
            if master is not None:
                os.close(master)

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

    def test_nonterminal_install_completes_without_disclosing_pairing_code(self):
        code = self.write_pairing_command()
        reference = "ghcr.io/wontonapp/wonton-core@sha256:" + "a" * 64

        result = self.run_install(WONTON_CORE_IMAGE=reference)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.env()["WONTON_CORE_IMAGE"], reference)
        self.assertNotIn(code, result.stdout + result.stderr)

    def test_terminal_output_starts_pairing_without_terminal_stdin(self):
        code = self.write_pairing_command()
        reference = "ghcr.io/wontonapp/wonton-core@sha256:" + "a" * 64

        result = self.run_install(terminal=True, WONTON_CORE_IMAGE=reference)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.env()["WONTON_CORE_IMAGE"], reference)
        self.assertIn(code, result.stdout)

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
        self.digests["ghcr.io/wontonapp/wonton-core:beta"] = [
            "ghcr.io/wontonapp/wonton-core@sha256:" + "e" * 64
        ]

        result = self.run_install(entry=False, WONTON_CORE_IMAGE="ghcr.io/wontonapp/wonton-core:beta")

        self.assertNotEqual(result.returncode, 0)
        log = (self.state / "log").read_text()
        self.assertIn("init", log)
        self.assertFalse((self.install_dir / ".env").exists())

    def test_none_mode_acme_coordinates_are_written_to_the_install_env(self):
        self.digests["ghcr.io/wontonapp/wonton-core:beta"] = [
            "ghcr.io/wontonapp/wonton-core@sha256:" + "f" * 64
        ]

        result = self.run_install(
            entry=False,
            WONTON_CORE_IMAGE="ghcr.io/wontonapp/wonton-core:beta",
            WONTON_MATRIX_NAME_MODE="none",
            WONTON_ACME_DIRECTORY="https://pebble.test/dir",
            WONTON_ACME_ACCEPT_TERMS="true",
            WONTON_ACME_RESOLVERS="192.0.2.53:53",
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        env = self.env()
        # none 模式没有 settings.json：坐标写进 .env，核心运行时才拿得到（ADR-0089 第 3 节）。
        self.assertEqual(env["WONTON_ACME_DIRECTORY"], "https://pebble.test/dir")
        self.assertEqual(env["WONTON_ACME_ACCEPT_TERMS"], "true")
        self.assertEqual(env["WONTON_ACME_RESOLVERS"], "192.0.2.53:53")

    def test_none_mode_install_defaults_the_acme_directory(self):
        # 不用名字的安装没有 entry/settings.json，运行时的 ACME 目录只能从环境读（clients/domain.rs 的 acme()）：
        # 没给就落到 Let's Encrypt 正式地址，别让新用户去填一个看不懂的变量。订户协议不读环境，
        # 由主人在 App 勾选，勾之前核心不联系 CA。
        self.digests["ghcr.io/wontonapp/wonton-core:stable"] = [
            "ghcr.io/wontonapp/wonton-core@sha256:" + "9" * 64
        ]
        result = self.run_install(entry=False, WONTON_MATRIX_NAME_MODE="none")

        self.assertEqual(result.returncode, 0, result.stderr)
        env = self.env()
        self.assertEqual(
            env["WONTON_ACME_DIRECTORY"],
            "https://acme-v02.api.letsencrypt.org/directory",
        )
        self.assertNotIn("WONTON_ACME_ACCEPT_TERMS", env)

    def test_none_mode_keeps_an_explicit_acme_directory(self):
        # 测试、内部 CA 用别的目录：默认值不能盖掉给的。
        self.digests["ghcr.io/wontonapp/wonton-core:stable"] = [
            "ghcr.io/wontonapp/wonton-core@sha256:" + "8" * 64
        ]
        result = self.run_install(
            entry=False,
            WONTON_MATRIX_NAME_MODE="none",
            WONTON_ACME_DIRECTORY="https://pebble.test/dir",
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.env()["WONTON_ACME_DIRECTORY"], "https://pebble.test/dir")

    def test_rerun_without_the_name_mode_keeps_a_nameless_install_going(self):
        # 升级就是再跑一次（README 的“退回”命令只带 digest），不给任何入口变量：init 容器经 --env-file 读到记下的
        # 模式和证书坐标，none 安装不会退回去要名字要的东西。
        self.digests["ghcr.io/wontonapp/wonton-core:stable"] = [
            "ghcr.io/wontonapp/wonton-core@sha256:" + "7" * 64
        ]
        result = self.run_install(entry=False, WONTON_MATRIX_NAME_MODE="none")
        self.assertEqual(result.returncode, 0, result.stderr)

        result = self.run_install(entry=False)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            self.env()["WONTON_ACME_DIRECTORY"],
            "https://acme-v02.api.letsencrypt.org/directory",
        )

    def test_readme_install_command_matches_the_script_default_image(self):
        # 公开文档指向的仓库和脚本默认镜像必须是同一个命名空间：issue #143 就是一边换了一边没换。
        readme = (SCRIPT.parent / "README.md").read_text()
        command = next(
            line for line in readme.splitlines() if "install.sh" in line and "curl" in line
        )
        url = command.split()[2]
        self.assertRegex(url, r"^https://raw\.githubusercontent\.com/[^/]+/wonton-install/main/install\.sh$")
        owner = url.split("/")[3]
        self.assertIn(f"ghcr.io/{owner}/wonton-core:stable", SCRIPT.read_text())

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

    def test_relay_fragment_and_vapid_survive_a_rerun(self):
        self.digests["ghcr.io/wontonapp/wonton-core:stable"] = [
            "ghcr.io/wontonapp/wonton-core@sha256:" + "d" * 64
        ]
        relays = "https://token@relay.example.org#7843;n0"
        builtin = "https://relay.example.org#7843,n0"

        # QUIC 端口是 #端口：fits_env 不能把 # 当 shell 特殊字符挡掉。
        result = self.run_install(
            WONTON_RELAYS=relays,
            WONTON_BUILTIN_RELAYS=builtin,
            WONTON_PUSH_VAPID_KEY="cHJpdmF0ZQ",
            WONTON_PUSH_VAPID_SUBJECT="mailto:ops@example.com",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        env = self.env()
        self.assertEqual(env["WONTON_RELAYS"], relays)
        self.assertEqual(env["WONTON_BUILTIN_RELAYS"], builtin)

        # 升级重写 .env 时经 saved 读回，不用再给。
        result = self.run_install(entry=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        env = self.env()
        self.assertEqual(env["WONTON_RELAYS"], relays)
        self.assertEqual(env["WONTON_BUILTIN_RELAYS"], builtin)
        self.assertEqual(env["WONTON_PUSH_VAPID_KEY"], "cHJpdmF0ZQ")
        self.assertEqual(env["WONTON_PUSH_VAPID_SUBJECT"], "mailto:ops@example.com")

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
