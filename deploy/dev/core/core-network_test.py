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

# 假 docker：网络状态放在 $FAKE_NETS/<名字>/ 下（project、subnet、containers 三个文件），每条写命令记进 $FAKE_LOG。
# `docker network inspect` 按 -f 的模板回答；`network rm` 像真的那样，网络上还有容器（containers 文件非空）就失败；
# 别的子命令只管记日志。
STUBS = r'''
export WONTON_UNUSED=; export WONTON_DEV_INSTANCE="${WONTON_DEV_INSTANCE-test}"
export WONTON_DEV_SECRETS="$FAKE_NETS/secrets"
# 两个子网有没有重叠：真 Docker 就是按这个判 Pool overlaps 的。
overlaps() {
  python3 - "$1" "$2" <<'PY'
import ipaddress, sys
try:
    a = ipaddress.ip_network(sys.argv[1], strict=False)
    b = ipaddress.ip_network(sys.argv[2], strict=False)
except ValueError:
    sys.exit(1)
sys.exit(0 if a.overlaps(b) else 1)
PY
}
docker() {
  local sub name tmpl="" i subnet="" arg other other_subnet
  local -a labels=()
  case "${1:-}" in
    # 核心经 API 起的入口/连接器容器由 remove_entry_components 单独删，这里当作没有。
    ps) [[ " $* " == *"component="* ]] || cat "$FAKE_CONTAINERS" 2>/dev/null; return 0 ;;
    stop) printf 'stop %s\n' "$*" >>"$FAKE_LOG"; return 0 ;;
    rm) printf 'rmc %s\n' "$*" >>"$FAKE_LOG"
        for arg in "${@:2}"; do
          [[ $arg == -f ]] && continue
          for files in "$FAKE_CONTAINERS" "$FAKE_NETS"/*/containers; do
            [[ -f $files ]] || continue
            grep -vx -- "$arg" "$files" >"$files.new" || :; mv "$files.new" "$files"
          done
        done
        return 0 ;;
    network) ;;
    *) printf 'other %s\n' "$*" >>"$FAKE_LOG"; return 0 ;;
  esac
  sub="$2"
  case "$sub" in
    ls) for arg in "$FAKE_NETS"/*; do [[ -d $arg ]] && basename "$arg"; done ;;
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
      other="Error response from daemon: invalid pool request: Pool overlaps with other one on this address space"
      [[ -z ${FAKE_POOL_FULL:-} ]] || { echo "$other" >&2; return 1; }
      for arg in "$FAKE_NETS"/*; do
        other_subnet="$(cat "$arg/subnet" 2>/dev/null)" || continue
        [[ -z $subnet || -z $other_subnet ]] || ! overlaps "$subnet" "$other_subnet" || { echo "$other" >&2; return 1; }
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
      printf 'rm %s\n' "$name" >>"$FAKE_LOG"
      if [[ -s "$FAKE_NETS/$name/containers" ]]; then
        echo "Error response from daemon: error while removing network: network $name has active endpoints" >&2
        return 1
      fi
      rm -rf "$FAKE_NETS/$name"
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
        # 假容器：instance_containers 从这里读，`docker rm` 清空；挂在网络上的记在网络自己的 containers 文件里。
        self.containers = self.temp / "containers"
        self.containers.touch()
        self.log = self.temp / "log"
        self.log.touch()

    def run_dev(self, *args, env_extra=None, execute=False, **extra):
        """source 后调用（默认）或按子命令真跑一遍：真跑时桩经 BASH_ENV 进去，脚本里的子进程也拿到同一套。"""
        env = {
            "PATH": os.environ["PATH"],
            "FAKE_CONTAINERS": str(self.containers),
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

    def seed_network(self, name=NETWORK, project=PROJECT, subnet="10.214.9.0/24"):
        (self.nets / name).mkdir(parents=True)
        (self.nets / name / "project").write_text(project)
        (self.nets / name / "subnet").write_text(subnet)

    def attach(self, container, network=NETWORK):
        """一个容器挂到网络上，并算作本实例的容器。"""
        self.containers.write_text(f"{container}\n")
        (self.nets / network / "containers").write_text(f"{container}\n")

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

    def test_remove_fails_while_containers_are_attached(self):
        # 插件容器还挂在上面时 Docker 拒删：要报错让调用方知道，不能打了话就返回 0。
        self.seed_network()
        self.attach("c0ffee")
        result = self.run_dev("remove_core_network")
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertTrue((self.nets / NETWORK).exists())
        self.assertIn(NETWORK, result.stderr)
        self.assertIn("down", result.stderr)

    def test_create_names_the_network_it_clashes_with(self):
        self.seed_network(name="wonton-other_core", project="wonton-other", subnet="10.214.8.0/24")
        result = self.run_dev("ensure_core_network", WONTON_DEV_CORE_SUBNET="10.214.8.128/25")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("wonton-other_core", result.stderr)
        self.assertIn("10.214.8.0/24", result.stderr)
        self.assertIn("换一个子网", result.stderr)
        self.assertIn("down", result.stderr)
        self.assertEqual([line for line in self.created() if line.endswith(NETWORK)], [])

    def test_full_default_pool_asks_for_a_subnet(self):
        # 不设子网时由 Docker 从默认池里挑，池满同样报 Pool overlaps：那时候要让人自己指定子网。
        result = self.run_dev("ensure_core_network", FAKE_POOL_FULL="1")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("WONTON_DEV_CORE_SUBNET", result.stderr)
        self.assertIn("down", result.stderr)

    def test_down_removes_plugin_containers_before_the_network(self):
        # 插件容器挂在 core 网络上：不先删它们，网络删不掉（#187 复审第 1 条）。
        self.seed_network()
        self.attach("c0ffee")
        result = self.run_dev("down", env_extra={"WONTON_INSTALL_DIR": "/nonexistent"}, execute=True)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("rmc rm -f c0ffee", self.log.read_text())
        log = self.log.read_text().splitlines()
        self.assertIn(f"rm {NETWORK}", log)
        self.assertLess(log.index("rmc rm -f c0ffee"), log.index(f"rm {NETWORK}"), log)
        self.assertFalse((self.nets / NETWORK).exists())

    def test_down_fails_when_the_network_cannot_be_removed(self):
        # 默认项目没有实例名，插件容器 down 不删：网络跟着留，down 要返回非零，并给出删容器的命令。
        default = "wonton-dev-core_core"
        self.seed_network(name=default, project="wonton-dev-core")
        self.attach("c0ffee", network=default)
        result = self.run_dev(
            "down",
            env_extra={"WONTON_INSTALL_DIR": "/nonexistent", "WONTON_DEV_INSTANCE": "", "WONTON_DEV_PROJECT": "wonton-dev-core"},
            execute=True,
        )
        self.assertNotEqual(result.returncode, 0, result.stderr)
        self.assertIn("docker rm -f c0ffee", result.stdout)
        self.assertIn(default, result.stderr)
        self.assertTrue((self.nets / default).exists())

    def test_reset_removes_plugin_containers_before_the_network(self):
        self.seed_network()
        self.attach("c0ffee")
        result = self.run_dev("reset", env_extra={"WONTON_INSTALL_DIR": "/nonexistent"}, execute=True)

        self.assertEqual(result.returncode, 0, result.stderr)
        log = self.log.read_text().splitlines()
        self.assertLess(log.index("rmc rm -f c0ffee"), log.index(f"rm {NETWORK}"), log)
        self.assertFalse((self.nets / NETWORK).exists())

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
        self.assertLess(
            log.index(next(line for line in log if line.startswith("create "))),
            next(i for i, line in enumerate(log) if "up -d --build --wait" in line),
            log,
        )

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
