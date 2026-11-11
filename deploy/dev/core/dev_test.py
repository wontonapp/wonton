#!/usr/bin/env python3
"""dev.sh 的 logs、diag：source 真的 dev.sh，只把 docker/compose/api 换成脚本，不连 Docker、不联网。

假 docker 只回答这几条要问的事；需要的结果由环境变量给（FAKE_DNS、FAKE_PROXY、FAKE_API）。
"""

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).parent
DEV_SH = HERE / "dev.sh"

SETTINGS = (
    '{"url":"http://host.docker.internal:7899","source":"settings","provider":null,'
    '"plugin_path":null,"core_path":null,"host_cidrs":[],"direct_cidrs":[],"notices":[],"restarts":[]}'
)
BAD_SETTINGS = SETTINGS.replace(":7899", ":1")
NOPROXY = SETTINGS.replace('"url":"http://host.docker.internal:7899"', '"url":null').replace(
    '"source":"settings"', '"source":"none"'
)

# 插进来替换的桩：compose、docker、api，以及避开环境和文件的 need_env。
STUBS = r'''
export WONTON_DEV_INSTANCE="${WONTON_DEV_INSTANCE-test}"
if [[ -n "$WONTON_DEV_INSTANCE" ]]; then core=wonton-test--core; else core=wonton-dev-core; fi
names() {
  if [[ -n "$WONTON_DEV_INSTANCE" ]]; then
    printf 'wonton-test--app-web--web\nwonton-test--app-web--web--bot\nwonton-test--db--db\n'
  else
    printf 'wonton-dev-core\nwonton-dev-core-hydra-1\nwonton-app-web--web\n'
  fi
}
need_env() { :; }
compose() {
  case "$*" in
    *"config --services"*) printf 'core\nhydra\nhydra-migrate\ntusd\ntuwunel\n' ;;
    *"logs"*) printf 'core-logs:%s\n' "${@: -1}" ;;
  esac
}
docker() {
  case "${1:-}" in
    ps)
      # clean-data 的“栈 down 没有”检查用 ps -q 按标签数容器；FAKE_RUNNING_STACKS 非空时当作有在跑的。
      if [[ " $* " == *" -q "* ]]; then
        [[ -n "${FAKE_RUNNING_STACKS:-}" ]] && printf 'c1\nc2\n'
      else
        names
      fi ;;
    run)
      # 模拟一次性容器：wipe_data 的 sh -c 整段脚本换算；clean_data 的 --entrypoint rm 把镜像后的参数换算后执行，
      # 挂载点（-v <宿主>:<容器路径>）里的容器路径换成宿主路径，让删除真的发生。
      local -a args=("${@:2}") cmd=()
      local i entry="" mount="" point="" script="" started=0
      for i in "${!args[@]}"; do
        case ${args[i]} in
          --entrypoint) entry="${args[i + 1]}" ;;
          -v) mount="${args[i + 1]}"; point="${mount#*:}" ;;
          -c) script="${args[i + 1]}" ;;
        esac
      done
      if [[ -n $script ]]; then
        bash -c "${script//${point}/${mount%%:*}}"
      else
        for i in "${!args[@]}"; do
          (( started )) && cmd+=("${args[i]//$point/${mount%%:*}}")
          [[ ${args[i]} == "$curl_image" ]] && started=1
        done
        if [[ -n $entry ]]; then "$entry" "${cmd[@]}"; else "${cmd[@]}"; fi
      fi
      ;;
    logs) printf 'plugin-logs:%s\n' "$*" ;;
    inspect) printf '%s\n' "${FAKE_RUNNING:-true}" ;;
    exec)
      shift
      case "$1" in
        *--core)
          case "${2:-}" in
            getent)
              [[ -n "${FAKE_DNS:-}" ]] || exit 2
              printf '%s\n' "$FAKE_DNS"
              ;;
            timeout) [[ "${FAKE_PROXY:-ok}" == ok ]] ;;
          esac
          ;;
        *)
          if [[ "${2:-}" == sh ]]; then
            printf 'http_proxy=%s\nno_proxy=%s\n' "${FAKE_HTTP_PROXY-}" "${FAKE_NO_PROXY-.wonton.internal}"
            [[ -z "${FAKE_HTTP_PROXY-}" || -n "${FAKE_NO_PROXY-}" ]]
          fi
          ;;
      esac
      ;;
  esac
}
api() {
  [[ "${FAKE_API:-ok}" == ok ]] || return 22
  printf '%s\n' "$FAKE_SETTINGS"
}
'''


class DevScriptTest(unittest.TestCase):
    def setUp(self):
        self.temp = Path(tempfile.mkdtemp(prefix="dev-script-"))
        self.addCleanup(shutil.rmtree, self.temp)

    def run_dev(self, *args, dev_sh=None, **extra):
        env = {
            "PATH": os.environ["PATH"],
            "HOME": str(self.temp),
            "WONTON_DEV_PROJECT": "wonton-test",
            "WONTON_DEV_DATA": str(self.temp / "data"),
            "WONTON_DEV_STATE": str(self.temp / "state"),
            **extra,
        }
        script = f'source "$1"\n{STUBS}\n' + " ".join(args)
        return subprocess.run(
            ["bash", "-c", script, "bash", str(dev_sh or DEV_SH)],
            env=env, capture_output=True, text=True, timeout=60,
        )

    def fake_core(self):
        """dev.sh 的副本放在假的检出树里：clean-data 的路径守卫拿 $here/..（deploy/dev）当边界，
        数据目录要在它下面，测试得有自己的一份 deploy/dev，不往真检出里写。"""
        core = self.temp / "deploy/dev/core"
        core.mkdir(parents=True)
        shutil.copy(DEV_SH, core / "dev.sh")
        shutil.copy(HERE / "diag.sh", core / "diag.sh")
        return core

    def test_logs_follows_core_service(self):
        result = self.run_dev("logs", "hydra")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("core-logs:hydra", result.stdout)

    def test_logs_works_without_instance(self):
        result = self.run_dev("logs", "web", WONTON_DEV_PROJECT="wonton-dev-core", WONTON_DEV_INSTANCE="")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("wonton-app-web--web", result.stdout)

    def test_logs_follows_plugin_service(self):
        result = self.run_dev("logs", "web")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("wonton-test--app-web--web", result.stdout)

    def test_unknown_service_lists_candidates(self):
        result = self.run_dev("logs", "nope")
        self.assertEqual(result.returncode, 1)
        self.assertIn("没有叫 nope 的服务", result.stderr)
        self.assertIn("hydra", result.stderr)
        self.assertIn("web", result.stderr)

    def test_diag_passes(self):
        result = self.run_dev(
            "diag", FAKE_SETTINGS=SETTINGS, FAKE_DNS="10.213.0.2 auth.wonton.internal"
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("都通了", result.stdout)
        self.assertIn("http_proxy=", result.stdout)

    def test_diag_without_proxy_says_so(self):
        result = self.run_dev(
            "diag", FAKE_SETTINGS=NOPROXY, FAKE_DNS="10.213.0.2 auth.wonton.internal"
        )
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("没有配出口代理", result.stdout)

    def test_diag_reports_each_failure_and_keeps_going(self):
        result = self.run_dev("diag", FAKE_SETTINGS=BAD_SETTINGS, FAKE_PROXY="fail", FAKE_DNS="")
        self.assertEqual(result.returncode, 1)
        self.assertIn("auth.wonton.internal", result.stdout)
        self.assertIn("失败（退出码", result.stdout)
        self.assertIn("连不上：超时或被拒绝", result.stdout)
        self.assertIn("no_proxy=", result.stdout, "前面的项失败后后面的项照样跑")
        self.assertIn("2 项失败", result.stdout)

    def test_diag_flags_proxy_without_no_proxy(self):
        result = self.run_dev(
            "diag",
            FAKE_SETTINGS=SETTINGS,
            FAKE_DNS="10.213.0.2 auth.wonton.internal",
            FAKE_HTTP_PROXY="http://host.docker.internal:7899",
            FAKE_NO_PROXY="",
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("有 http_proxy 却没有 no_proxy", result.stdout)

    def test_diag_settings_failure_does_not_stop_the_rest(self):
        result = self.run_dev("diag", FAKE_API="fail")
        self.assertEqual(result.returncode, 1)
        self.assertIn("读不到设置", result.stdout)
        self.assertIn("http_proxy=", result.stdout, "接口失败后后面的项照样跑")

    def test_clean_data_removes_data_dir_inside_deploy_dev(self):
        core = self.fake_core()
        data = core / ".data"
        (data / "core").mkdir(parents=True)
        (data / "core" / "db").write_text("x")
        (data / ".hidden").write_text("x")
        result = self.run_dev("clean_data", dev_sh=core / "dev.sh", WONTON_DEV_DATA=str(data))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(data.exists(), "数据目录整个删掉")
        self.assertIn("已删掉数据目录", result.stdout)

    def test_clean_data_refuses_while_stack_is_running(self):
        core = self.fake_core()
        data = core / ".data"
        (data / "core").mkdir(parents=True)
        result = self.run_dev(
            "clean_data", dev_sh=core / "dev.sh", WONTON_DEV_DATA=str(data), FAKE_RUNNING_STACKS="1"
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("还有在跑的容器", result.stderr)
        self.assertTrue(data.exists(), "栈在跑时不能删数据目录")

    def test_clean_data_refuses_outside_deploy_dev(self):
        outside = self.temp / "elsewhere" / "data"
        outside.mkdir(parents=True)
        result = self.run_dev("clean_data", WONTON_DEV_DATA=str(outside))
        self.assertEqual(result.returncode, 1)
        self.assertIn("不在", result.stderr)
        self.assertTrue(outside.exists(), "检出外的目录不动")

    def test_clean_data_refuses_deploy_dev_itself(self):
        core = self.fake_core()
        deploy = self.temp / "deploy" / "dev"
        result = self.run_dev("clean_data", dev_sh=core / "dev.sh", WONTON_DEV_DATA=str(deploy))
        self.assertEqual(result.returncode, 1)
        self.assertIn("不在", result.stderr)
        self.assertTrue(deploy.exists())


if __name__ == "__main__":
    unittest.main()
