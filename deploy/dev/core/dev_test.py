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
    ps) names ;;
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

    def run_dev(self, *args, **extra):
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
            ["bash", "-c", script, "bash", str(DEV_SH)],
            env=env, capture_output=True, text=True, timeout=60,
        )

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


if __name__ == "__main__":
    unittest.main()
