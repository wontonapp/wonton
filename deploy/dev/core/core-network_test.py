#!/usr/bin/env python3
"""dev.sh 的 core 网络：由脚本建、只建一次，已存在就复用（#187）。

背景：compose 自己管网络时，只要渲染出来的网络配置和现有的对不上（比如 WONTON_DEV_CORE_SUBNET 换过、
这次没设）就把网络删了重建；重建后它把已经跑着的容器接回去时不带别名，`hydra.wonton.internal` 这些名字
就丢了，核心再也解析不到。所以网络改成 external，由 dev.sh 建和删，compose 只把容器接上去。

每个用例都 source 真的 dev.sh，docker 换成记日志、存在目录里的假实现，不碰真的 Docker。
"""

import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).parent
DEV_SH = HERE / "dev.sh"
COMPOSE = HERE / "compose.yaml"
PROJECT = "wonton-test"
NETWORK = f"{PROJECT}_core"

# 假 docker：网络状态放在 $FAKE_NETS/<名字>/ 下（project、subnet 两个文件），每条写命令记进 $FAKE_LOG。
# `docker network inspect` 按 -f 的模板回答，别的子命令只管记日志。
STUBS = r'''
export WONTON_DEV_INSTANCE="${WONTON_DEV_INSTANCE-test}"
export WONTON_DEV_SECRETS="$FAKE_NETS/secrets"
docker() {
  local sub name tmpl="" i subnet="" arg
  local -a labels=()
  [[ "${1:-}" == network ]] || { printf 'other %s\n' "$*" >>"$FAKE_LOG"; return 0; }
  sub="$2"
  case "$sub" in
    inspect)
      [[ "${3:-}" == -f ]] && tmpl="$4"
      name="${@: -1}"
      [[ -d "$FAKE_NETS/$name" ]] || { echo "Error: No such network: $name" >&2; return 1; }
      printf 'inspect %s %s\n' "$name" "$tmpl" >>"$FAKE_LOG"
      case "$tmpl" in
        *IPAM*) cat "$FAKE_NETS/$name/subnet" 2>/dev/null ;;
        *Labels*) cat "$FAKE_NETS/$name/project" 2>/dev/null ;;
      esac
      ;;
    create)
      for i in "${@:3}"; do
        if [[ ${pending:-} == subnet ]]; then subnet="$i"; pending=; unset pending
        elif [[ ${pending:-} == label ]]; then labels+=("$i"); pending=; unset pending
        elif [[ $i == --subnet || $i == --label ]]; then pending="${i#--}"
        else name="$i"; fi
      done
      mkdir -p "$FAKE_NETS/$name"
      [[ -n $subnet ]] && printf '%s' "$subnet" >"$FAKE_NETS/$name/subnet"
      for arg in ${labels[@]+"${labels[@]}"}; do
        [[ $arg == com.docker.compose.project=* ]] && printf '%s' "${arg#*=}" >"$FAKE_NETS/$name/project"
      done
      printf 'create %s\n' "$*" >>"$FAKE_LOG"
      echo "$name"
      ;;
    rm)
      name="${@: -1}"
      [[ -d "$FAKE_NETS/$name" ]] || { echo "Error: No such network: $name" >&2; return 1; }
      rm -rf "$FAKE_NETS/$name"
      printf 'rm %s\n' "$name" >>"$FAKE_LOG"
      echo "$name"
      ;;
  esac
}
'''


class CoreNetworkTest(unittest.TestCase):
    def setUp(self):
        self.temp = Path(tempfile.mkdtemp(prefix="core-network-"))
        self.addCleanup(shutil.rmtree, self.temp)
        self.nets = self.temp / "nets"
        self.nets.mkdir()
        self.log = self.temp / "log"
        self.log.touch()

    def run_dev(self, *args, env_extra=None, execute=False, **extra):
        """source 后调用（默认）或按子命令真跑一遍：真跑时桩经 BASH_ENV 进去，脚本里的子进程也拿到同一套。"""
        env = {
            "PATH": os.environ["PATH"],
            "HOME": str(self.temp),
            "WONTON_DEV_PROJECT": PROJECT,
            "WONTON_DEV_DATA": str(self.temp / "data"),
            "FAKE_NETS": str(self.nets),
            "FAKE_LOG": str(self.log),
            **(env_extra or {}),
            **(extra or {}),
        }
        if execute:
            stub = self.temp / "stubs.sh"
            stub.write_text(STUBS + "\nss() { :; }\n")
            return subprocess.run(
                ["bash", str(DEV_SH), *args],
                env={**env, "BASH_ENV": str(stub)},
                capture_output=True, text=True, timeout=120,
            )
        script = f'source "$1"\n{STUBS}\n' + "\n".join(args) + "\n"
        return subprocess.run(
            ["bash", "-c", script, "bash", str(DEV_SH)],
            env=env, capture_output=True, text=True, timeout=60,
        )

    def seed_network(self, project=PROJECT, subnet="10.214.9.0/24"):
        (self.nets / NETWORK).mkdir()
        (self.nets / NETWORK / "project").write_text(project)
        (self.nets / NETWORK / "subnet").write_text(subnet)

    def created(self):
        return [line for line in self.log.read_text().splitlines() if line.startswith("create ")]

    def test_missing_network_is_created_with_subnet_and_compose_labels(self):
        result = self.run_dev(
            "if ensure_core_network; then echo rc=0; else echo rc=$?; fi",
            WONTON_DEV_CORE_SUBNET="10.214.9.0/24",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("rc=0", result.stdout)
        created = self.created()
        self.assertEqual(len(created), 1, self.log.read_text())
        self.assertIn(f"--subnet 10.214.9.0/24", created[0])
        self.assertIn(f"--label com.docker.compose.project={PROJECT}", created[0])
        self.assertIn("--label com.docker.compose.network=core", created[0])
        self.assertTrue(created[0].endswith(NETWORK), created[0])

    def test_no_subnet_means_docker_picks_one(self):
        result = self.run_dev("ensure_core_network")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("--subnet", self.created()[0])

    def test_existing_network_is_reused_not_recreated(self):
        self.seed_network()
        result = self.run_dev("ensure_core_network", WONTON_DEV_CORE_SUBNET="10.214.9.0/24")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.created(), [])
        self.assertNotIn("rm", self.log.read_text())

    def test_existing_network_with_another_subnet_is_kept_with_a_note(self):
        self.seed_network(subnet="10.214.9.0/24")
        result = self.run_dev("ensure_core_network", WONTON_DEV_CORE_SUBNET="10.214.8.0/24")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.created(), [])
        self.assertIn("沿用", result.stderr)
        self.assertIn("10.214.9.0/24", result.stderr)

    def test_network_of_another_project_is_refused(self):
        self.seed_network(project="wonton-other")
        result = self.run_dev("ensure_core_network")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.created(), [])
        self.assertIn("wonton-other", result.stderr)

    def test_remove_deletes_the_network(self):
        self.seed_network()
        result = self.run_dev("remove_core_network")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.nets / NETWORK).exists())
        self.assertIn(f"rm {NETWORK}", self.log.read_text())

    def test_remove_without_network_succeeds(self):
        result = self.run_dev("remove_core_network")
        self.assertEqual(result.returncode, 0, result.stderr)


    def test_down_removes_the_network_after_compose(self):
        self.seed_network()
        result = self.run_dev("down", env_extra={"WONTON_INSTALL_DIR": "/nonexistent"}, execute=True)

        self.assertEqual(result.returncode, 0, result.stderr)
        log = self.log.read_text().splitlines()
        self.assertIn(f"rm {NETWORK}", log)
        self.assertLess(
            max(i for i, line in enumerate(log) if line.endswith("down")),
            log.index(f"rm {NETWORK}"),
            log,
        )

    def test_up_creates_the_network_before_compose(self):
        result = self.run_dev(
            "up",
            env_extra={
                "WONTON_INSTALL_DIR": "/nonexistent",
                "WONTON_BUILD_COMMIT": "test",
                "WONTON_DEV_OWNER_PROFILE": "off",
                "WONTON_DEV_PORT": "38480",
                "WONTON_DEV_TUNNEL_PORT": "38482",
                "WONTON_DEV_CORE_SUBNET": "10.214.9.0/24",
            },
            execute=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        log = self.log.read_text().splitlines()
        self.assertTrue(any(line.startswith("create ") for line in log), log)
        self.assertTrue(any("up -d --build --wait" in line for line in log), log)

class ComposeFileTest(unittest.TestCase):
    """compose 文件把 core 网络声明成 external：网络归 dev.sh，compose 不再建、也不再重建它。"""

    def core_block(self):
        text = COMPOSE.read_text()
        match = re.search(r"^networks:\n(?:.*\n)*?^  core:\n((?:    .*\n|[ \t]*\n)*)", text, re.M)
        self.assertIsNotNone(match, "compose.yaml 里没有顶层的 networks.core")
        return match.group(1)

    def test_core_network_is_external(self):
        self.assertIn("external: true", self.core_block())

    def test_core_network_name_follows_the_project(self):
        self.assertIn(
            "name: ${WONTON_DEV_PROJECT:-wonton-dev-core}_core",
            self.core_block(),
        )


class ComposeCommandTest(unittest.TestCase):
    """compose 调用只带 compose.yaml：子网写在线建的网络上，不再拼 override 文件。"""

    def test_subnet_does_not_add_an_override_file(self):
        temp = Path(tempfile.mkdtemp(prefix="core-compose-"))
        self.addCleanup(shutil.rmtree, temp)
        log = temp / "log"
        log.touch()
        env = {
            "PATH": os.environ["PATH"],
            "HOME": str(temp),
            "WONTON_DEV_PROJECT": PROJECT,
            "WONTON_DEV_DATA": str(temp / "data"),
            "WONTON_DEV_CORE_SUBNET": "10.214.9.0/24",
            "FAKE_LOG": str(log),
        }
        script = 'source "$1"\ndocker() { printf "%s\\n" "$*" >>"$FAKE_LOG"; }\ncompose config\n'
        result = subprocess.run(
            ["bash", "-c", script, "bash", str(DEV_SH)],
            env=env, capture_output=True, text=True, timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        files = re.findall(r"-f (\S+)", log.read_text())
        self.assertEqual(files, [str(COMPOSE)])


if __name__ == "__main__":
    unittest.main()
