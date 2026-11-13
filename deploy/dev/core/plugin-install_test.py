#!/usr/bin/env python3
"""Exercise install's pipeline and exit semantics without Docker or network access."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).parent

HARNESS = r'''
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$PLUGIN_COMMANDS"
preview_package() { printf '{}'; }
preview_read() { GRANTS=; }
die() { printf '%s\n' "$*" >&2; exit 1; }
api() {
  [[ "$1" == POST && "$2" == '/_wonton/api/plugins?grant=' ]] || return 99
  cat >/dev/null
  printf '%s' "$API_OUTPUT"
  printf '%s' "$API_ERROR" >&2
  return "$API_STATUS"
}
if [[ "$PACKAGE_STATUS" != 0 ]]; then
  package() { return "$PACKAGE_STATUS"; }
fi
install "$1" --yes
printf 'after-install\n'
'''


class PluginInstallTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="plugin-install-")
        self.addCleanup(temporary.cleanup)
        self.core = Path(temporary.name) / "checkout with spaces" / "deploy/dev/core"
        self.core.mkdir(parents=True)
        self.dev = self.core / "dev.sh"
        self.dev.write_text(HARNESS)
        self.package = self.core / "compose.yaml"
        self.package.write_text("services: {}\n")
        self.status_command = str(self.dev).replace(" ", r"\ ") + " status"

    def run_install(self, api_status, package_status=0, output="", error=""):
        return subprocess.run(
            ["bash", str(self.dev), str(self.package)],
            env={
                "PATH": os.environ["PATH"],
                "PLUGIN_COMMANDS": str((HERE / "plugin-cmds.sh").resolve()),
                "API_STATUS": str(api_status),
                "PACKAGE_STATUS": str(package_status),
                "API_OUTPUT": output,
                "API_ERROR": error,
            },
            capture_output=True, text=True, timeout=10,
        )

    def test_timeout_preserves_error_and_guides_status_query(self):
        for package_status in (0, 141):
            with self.subTest(package_status=package_status):
                raw_error = "curl: (28) Operation timed out\n"
                result = self.run_install(28, package_status, output="partial", error=raw_error)
                self.assertEqual(result.returncode, 28)
                self.assertEqual(result.stdout, "partial")
                self.assertTrue(result.stderr.startswith(raw_error))
                self.assertIn(self.status_command, result.stderr)

    def test_other_api_failures_preserve_output_without_timeout_guidance(self):
        for status in (7, 22, 125):
            with self.subTest(status=status):
                raw_error = f"transport error {status}\n"
                result = self.run_install(status, output='{"error":"denied"}', error=raw_error)
                self.assertEqual(result.returncode, status)
                self.assertEqual(result.stdout, '{"error":"denied"}')
                self.assertEqual(result.stderr, raw_error)

    def test_package_failure_is_not_a_curl_timeout(self):
        result = self.run_install(0, package_status=28)
        self.assertEqual(result.returncode, 28)
        self.assertEqual(result.stdout, "")
        self.assertEqual(result.stderr, "")

    def test_success_preserves_output_and_continues(self):
        result = self.run_install(0, output='{"id":"app.example"}', error="diagnostic\n")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, '{"id":"app.example"}\nafter-install\n')
        self.assertEqual(result.stderr, "diagnostic\n")


if __name__ == "__main__":
    unittest.main()
