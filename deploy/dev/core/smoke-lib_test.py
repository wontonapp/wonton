#!/usr/bin/env python3
"""smoke.sh 分段运行（smoke-lib.sh 的 step、SMOKE_UNTIL、SMOKE_ONLY）的行为测试：不起栈、不连 Docker。

每个用例 source 真实的 smoke-lib.sh，按 smoke_steps 的顺序逐个调用 step，记下哪些步骤跑了、在哪里停下。
"""
import os
import subprocess
import unittest
from pathlib import Path

HERE = Path(__file__).parent
LIB = HERE / "smoke-lib.sh"

RUN_ALL = """set -euo pipefail
source "$1"
for s in "${smoke_steps[@]}" ${EXTRA_STEP:-}; do
  if step "$s" "标题 $s" >/dev/null; then echo "$s"; fi
done
echo END
"""


def run(**env_overrides):
    env = {k: v for k, v in os.environ.items() if not k.startswith("SMOKE_")}
    env.update(env_overrides)
    result = subprocess.run(
        ["bash", "-c", RUN_ALL, "bash", str(LIB)], env=env, capture_output=True, text=True, timeout=30
    )
    return result.returncode, result.stdout.split(), result.stderr


def all_steps():
    result = subprocess.run(
        ["bash", "-c", 'source "$1"; echo "${smoke_steps[@]}"', "bash", str(LIB)],
        capture_output=True, text=True, timeout=30, check=True,
    )
    return result.stdout.split()


class SmokeSteps(unittest.TestCase):
    def test_default_runs_every_step(self):
        self.assertEqual(run(), (0, all_steps() + ["END"], ""))

    def test_until_stops_after_the_named_step(self):
        steps = all_steps()
        rc, ran, _ = run(SMOKE_UNTIL="sso")
        self.assertEqual(rc, 0)
        self.assertEqual(ran, steps[: steps.index("sso") + 1])

    def test_only_runs_listed_steps(self):
        self.assertEqual(run(SMOKE_ONLY="whoami,core")[:2], (0, ["core", "whoami", "END"]))

    def test_unknown_step_name_fails_before_anything_runs(self):
        rc, ran, err = run(SMOKE_ONLY="helth")
        self.assertEqual((rc, ran), (1, []))
        self.assertIn("helth", err)

    def test_until_and_only_together_fail(self):
        self.assertEqual(run(SMOKE_UNTIL="core", SMOKE_ONLY="core")[:2], (1, []))

    def test_reaches_counts_until_and_only(self):
        script = 'source "$1"; for s in core hermes; do reaches "$s" && echo "$s"; done; true'
        for env, want in (({}, "core hermes"), ({"SMOKE_UNTIL": "sso"}, "core"),
                          ({"SMOKE_UNTIL": "hermes"}, "core hermes"), ({"SMOKE_ONLY": "hermes"}, "hermes")):
            full = {k: v for k, v in os.environ.items() if not k.startswith("SMOKE_")}
            full.update(env)
            out = subprocess.run(["bash", "-c", script, "bash", str(LIB)], env=full,
                                 capture_output=True, text=True, timeout=30, check=True).stdout.split()
            self.assertEqual(" ".join(out), want, env)

    def test_unregistered_step_in_script_fails(self):
        rc, ran, err = run(EXTRA_STEP="not-a-step")
        self.assertEqual(rc, 1)
        self.assertNotIn("END", ran)
        self.assertIn("not-a-step", err)


if __name__ == "__main__":
    unittest.main()
