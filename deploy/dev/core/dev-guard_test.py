#!/usr/bin/env python3
"""dev-guard：开发脚本和冒烟脚本不许碰正式安装的栈。

三道判据（项目名是 wonton、容器是 /opt/wonton 起的、当前目录在 /opt/wonton 里）各写一条用例，命中要求退出码 2、
在标准错误上说原因、且一条 docker 写命令都不发。正式安装目录用 WONTON_INSTALL_DIR 指到临时目录，本机装了真云吞
也不影响。假 docker 只记参数、只回答 ps 的两种问法。
"""

import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).parent
DEPLOY = HERE.parent
GUARD = HERE / "dev-guard.sh"

FAKE_DOCKER = """#!/usr/bin/env bash
printf '%s\\n' "$*" >>"$FAKE_DOCKER_LOG"
[[ -z ${FAKE_DOCKER_FAIL:-} ]] || exit 1
if [[ ${1:-} == ps ]]; then
  if [[ " $* " == *" --filter name="* ]]; then
    [[ -z ${FAKE_CONTAINER:-} ]] || printf '%s\\n' "$FAKE_CONTAINER"
  elif [[ " $* " == *"com.docker.compose.project="* ]] && [[ -n ${FAKE_PS_LABELS:-} ]]; then
    printf '%s\\n' "$FAKE_PS_LABELS"
  fi
fi
exit 0
"""

# 写命令的动词。假 docker 记的是整条参数，ps、inspect、version、ls 都不该命中。
WRITE = re.compile(r"\b(up|down|start|stop|restart|create|rm|run|build|tag)\b")


class DevGuardTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="dev-guard-"))
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.install = self.tmp / "opt/wonton"
        self.install.mkdir(parents=True)
        self.elsewhere = self.tmp / "elsewhere"
        self.elsewhere.mkdir()
        self.bin = self.tmp / "bin"
        self.bin.mkdir()
        docker = self.bin / "docker"
        docker.write_text(FAKE_DOCKER)
        docker.chmod(0o755)
        # 两个 PY 入口在拿名额之前断言探针二进制在；内容不重要。
        for name in ("fr_share_smoke", "fr_contacts_smoke", "share-smoke-relay", "core", "home"):
            binary = self.bin / name
            binary.write_text("#!/bin/sh\n")
            binary.chmod(0o755)
        # 名额脚本按 $HOME 找：换成假的，跑测试不占本机名额。
        self.home = self.tmp / "home"
        (self.home / "wonton-dev").mkdir(parents=True)
        slot = self.home / "wonton-dev/stack-slot.sh"
        slot.write_text("#!/usr/bin/env bash\ncase \"${1:-}\" in subnet) echo 10.214.7.0/24 ;; esac\n")
        slot.chmod(0o755)
        self.tmpdir = self.tmp / "tmp"
        self.tmpdir.mkdir()
        self.log = self.tmp / "docker.log"
        self.state = self.tmp / "state"
        self.env = {
            **os.environ,
            "PATH": f"{self.bin}:{os.environ['PATH']}",
            "WONTON_INSTALL_DIR": str(self.install),
            "FAKE_DOCKER_LOG": str(self.log),
            "WONTON_DEV_DATA": str(self.state / "data"),
            "WONTON_DEV_SECRETS": str(self.state / "secrets"),
            "WONTON_DEV_RELAY": str(self.tmp / "relay"),
            "WONTON_DEV_CORE_SUBNET": "10.214.7.0/24",
            "WONTON_CAPABILITY_CORE_IMAGE": "wonton-core:test",
            "WONTON_SHARE_SMOKE_BIN_DIR": str(self.bin),
            "WONTON_CORE_BINARY": str(self.bin / "core"),
            "HEAD_BINARY": str(self.bin / "core"),
            "MAIN_BINARY": str(self.bin / "core"),
            "WONTON_HOME_BINARY": str(self.bin / "home"),
            "DOCKER_HOST": "",
            "HOME": str(self.home),
            "TMPDIR": str(self.tmpdir),
        }
        for key in ("WONTON_DEV_PROJECT", "WONTON_DEV_INSTANCE", "SMOKE_ONLY", "COMPOSE_PROJECT_NAME"):
            self.env.pop(key, None)
        # entry-cloud 的 down 要先读 .runtime/stack.env；跑测试时别覆盖开发机上正在用的那一份。
        self.stack_env = DEPLOY / "entry-cloud/.runtime/stack.env"
        self.saved_stack_env = self.stack_env.read_bytes() if self.stack_env.exists() else None
        self.addCleanup(self.restore_stack_env)

    def restore_stack_env(self):
        if self.saved_stack_env is None:
            self.stack_env.unlink(missing_ok=True)
        else:
            self.stack_env.parent.mkdir(parents=True, exist_ok=True)
            self.stack_env.write_bytes(self.saved_stack_env)

    def shell(self, script, args=(), cwd=None, env=None):
        if self.log.exists():
            self.log.unlink()
        command = [str(script), *args] if script.suffix == ".sh" else ["python3", str(script), *args]
        if script.suffix == ".sh":
            command.insert(0, "bash")
        return subprocess.run(command, cwd=str(cwd or self.elsewhere), env={**self.env, **(env or {})},
                              capture_output=True, text=True, timeout=120)

    def writes(self):
        if not self.log.exists():
            return []
        return [line for line in self.log.read_text().splitlines() if WRITE.search(line)]

    def refused(self, result, reason):
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("dev-guard", result.stderr)
        self.assertIn(reason, result.stderr)
        self.assertEqual(self.writes(), [], result.stderr)

    def passed(self, result):
        self.assertNotIn("dev-guard", result.stderr)
        self.assertNotEqual(result.returncode, 2, result.stderr)

    def test_reasons(self):
        named = self.shell(GUARD, ["wonton"])
        self.refused(named, "正式安装的 compose 项目")

        label = f"{self.install}\t{self.install}/compose.yaml"
        labelled = self.shell(GUARD, ["wonton-dev-core"], env={"FAKE_PS_LABELS": label})
        self.refused(labelled, str(self.install))

        # 只有 config_files 标签（working_dir 为空的旧容器）也算。
        files_only = self.shell(GUARD, ["wonton-dev-core"], env={"FAKE_PS_LABELS": f"\t{self.install}/compose.yaml"})
        self.refused(files_only, str(self.install))

        here = self.shell(GUARD, ["wonton-dev-core"], cwd=self.install)
        self.refused(here, "当前目录")

        # 正式安装的冒烟自己操作那一套：--take-over 一律放行。
        self.assertNotIn("dev-guard", self.shell(GUARD, ["wonton", "--take-over"], cwd=self.install).stderr)

    def test_names_and_directories(self):
        for project in ("wonton-dev-core", "wonton-smoke", "wonton-e2exxxx"):
            with self.subTest(project=project):
                result = self.shell(GUARD, [project])
                self.assertEqual(result.returncode, 0, result.stderr)
        # 前缀比较：同级的 wonton2 目录不算正式安装目录。
        sibling = self.tmp / "opt/wonton2"
        sibling.mkdir()
        self.assertEqual(self.shell(GUARD, ["wonton-dev-core"], cwd=sibling).returncode, 0)
        # 读不进去的正式安装目录（本机真装的那份是 root 700）不能让别的目录全被判成“在里面”，
        # 也不能把它自己当成根目录。
        locked = self.tmp / "opt/locked"
        locked.mkdir()
        locked.chmod(0o000)
        self.addCleanup(locked.chmod, 0o700)
        locked_run = self.shell(GUARD, ["wonton-dev-core"], cwd=sibling, env={"WONTON_INSTALL_DIR": str(locked)})
        self.assertEqual(locked_run.returncode, 0, locked_run.stderr)
        self.assertNotIn("正式安装目录", locked_run.stderr)
        # 符号链接自环解析不出真路径（`-d` 为假），也按配置的路径比。
        loop = self.tmp / "loop"
        loop.symlink_to(loop)
        looped = self.shell(GUARD, ["wonton-dev-core"], cwd=sibling, env={"WONTON_INSTALL_DIR": str(loop)})
        self.assertEqual(looped.returncode, 0, looped.stderr)
        # 查不到 Docker 时不误伤：脚本本来就跑不起来，守卫不该再挡一层。
        broken = self.shell(GUARD, ["wonton-dev-core"], env={"FAKE_DOCKER_FAIL": "1"})
        self.assertEqual(broken.returncode, 0, broken.stderr)

    def test_install_dir_spellings_find_the_same_directory(self):
        # 正式安装目录可能是 root 700：cd 进不去。相对路径、带 ..、符号链接都得先规范化成真路径再比，
        # 否则三道判据全落空、守卫放行。
        wrapper = self.tmp / "enter-locked.sh"
        wrapper.write_text('#!/usr/bin/env bash\ncd "$1" || exit 99\nchmod 000 "$2"\nshift 2\nexec "$@"\n')
        wrapper.chmod(0o755)
        deeper = self.install / "sub"
        deeper.mkdir()
        link = self.tmp / "sym"
        link.symlink_to(self.install)
        self.addCleanup(self.install.chmod, 0o700)
        for spelling in ("..", f"{self.install}/sub/..", str(link)):
            with self.subTest(install_dir=spelling):
                self.install.chmod(0o700)
                result = self.shell(wrapper, [str(deeper), str(self.install), GUARD, "wonton-dev-core"],
                                    env={"WONTON_INSTALL_DIR": spelling})
                self.refused(result, os.path.realpath(self.install))

    def test_scripts_refuse(self):
        label = f"{self.install}\t{self.install}/compose.yaml"
        cases = [
            ("core/dev.sh", ["up"], "wonton", {}, "正式安装的 compose 项目"),
            ("core/dev.sh", ["down"], "wonton", {}, "正式安装的 compose 项目"),
            ("core/dev.sh", ["reset"], "wonton", {}, "正式安装的 compose 项目"),
            ("core/smoke.sh", [], None, {}, "当前目录", self.install),
            ("core/smoke-capabilities.sh", [], "wonton", {}, "正式安装的 compose 项目"),
            ("core/smoke-mailbox.sh", [], "wonton", {}, "正式安装的 compose 项目"),
            ("core/smoke-onboarding.sh", [], None, {}, "当前目录", self.install),
            ("core/smoke-sharing.sh", [], None, {"FAKE_PS_LABELS": label}, str(self.install)),
            ("core/smoke-internal-issuer.sh", [], None, {}, "当前目录", self.install),
            ("core/store/update.sh", [], None, {}, "当前目录", self.install),
            # 入口云的脚本先 cd 到检出自己，当前目录的判据在这里用不上，靠容器标签。
            ("entry-cloud/dev.sh", ["up"], None, {"FAKE_PS_LABELS": label}, str(self.install)),
            ("entry-cloud/prepare.py", [], None, {"FAKE_PS_LABELS": label}, str(self.install)),
            ("entry-cloud/smoke.py", [], None, {"FAKE_PS_LABELS": label}, str(self.install)),
            ("entry-cloud/upgrade-smoke.py", [], None, {"FAKE_PS_LABELS": label}, str(self.install)),
            # Tuwunel 开发栈的项目名写死在 compose.yaml，只能被当前目录这条判据挡住。
            ("matrix/dev.sh", ["up"], None, {}, "当前目录", self.install),
            ("matrix/dev.sh", ["down"], None, {}, "当前目录", self.install),
            ("matrix/dev.sh", ["reset"], None, {}, "当前目录", self.install),
        ]
        for relative, args, project, env, reason, *cwd in cases:
            with self.subTest(script=relative, args=args):
                end = {**env}
                if project:
                    end["WONTON_DEV_PROJECT"] = project
                if relative == "entry-cloud/dev.sh":
                    self.stack_env.parent.mkdir(parents=True, exist_ok=True)
                    self.stack_env.write_text("COMPOSE_PROJECT_NAME=wonton-entry-cloud\n")
                self.refused(self.shell(DEPLOY / relative, args, cwd=cwd[0] if cwd else None, env=end), reason)

    def test_entry_cloud_down_refuses(self):
        self.stack_env.parent.mkdir(parents=True, exist_ok=True)
        self.stack_env.write_text("COMPOSE_PROJECT_NAME=wonton-entry-cloud\n")
        result = self.shell(DEPLOY / "entry-cloud/dev.sh", ["down"], env={"FAKE_PS_LABELS": f"{self.install}\t"})
        self.refused(result, str(self.install))

    def test_dev_down_passes_and_writes(self):
        result = self.shell(HERE / "dev.sh", ["down"], env={"WONTON_DEV_PROJECT": "wonton-dev-core"})
        self.passed(result)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue([line for line in self.writes() if line.split()[-1] == "down"], self.writes())

    def test_smoke_only_against_running_stack(self):
        # SMOKE_ONLY 对着已经在跑的栈：项目就是它的，已经选定了，守卫放行。
        result = self.shell(HERE / "smoke.sh", env={"SMOKE_ONLY": "preflight", "WONTON_DEV_PROJECT": "wonton"})
        self.passed(result)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_upgrade_smoke_takes_over(self):
        # 升级冒烟故意用正式安装的项目名和 compose（name: wonton），主机上有没有真装由它自己查。
        result = self.shell(HERE / "smoke-upgrade.sh", env={"FAKE_PS_LABELS": f"{self.install}\t"})
        self.assertNotIn("dev-guard", result.stderr)
        self.assertNotEqual(result.returncode, 2, result.stderr)


if __name__ == "__main__":
    unittest.main()
