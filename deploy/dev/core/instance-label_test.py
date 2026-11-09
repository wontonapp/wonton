#!/usr/bin/env python3
"""dev.sh 按项目名生成实例标签的行为测试：python3 -m unittest 或直接运行。

每个用例都经 bash source 真实的 dev.sh，读它导出的 WONTON_DEV_INSTANCE（起栈时真正生效的值）。
背景见 #855：两个只差标点的项目名（wonton-chat-image / wonton-chatimage）过去得到同一个标签，
down 按标签清插件资源时会清到对方的。
"""
import os
import re
import subprocess
import unittest
from pathlib import Path

HERE = Path(__file__).parent
DEV_SH = HERE / "dev.sh"

VALID_LABEL = re.compile(r"^[a-z0-9]{1,16}$")


def dev_sh_script(instance: str | None) -> str:
    """source 后打印导出的 WONTON_DEV_INSTANCE。"""
    script = "source \"$1\"; printf '%s' \"${WONTON_DEV_INSTANCE-<unset>}\""
    if instance is None:
        return script
    return f"export WONTON_DEV_INSTANCE={instance}; {script}"


def exported_instance(project: str, instance: str | None = None) -> str:
    env = dict(os.environ)
    # 不让外面的环境冒充默认值：要测的就是 dev.sh 自己按项目名算出来的那个。
    env.pop("WONTON_DEV_PROJECT", None)
    env.pop("WONTON_DEV_INSTANCE", None)
    if project is not None:
        env["WONTON_DEV_PROJECT"] = project
    result = subprocess.run(
        ["bash", "-c", dev_sh_script(instance), "bash", str(DEV_SH)],
        env=env, capture_output=True, text=True, timeout=30,
    )
    if result.returncode != 0:
        raise AssertionError(f"dev.sh source 失败：{result.stderr}")
    return result.stdout


class InstanceLabelTest(unittest.TestCase):
    def test_punctuation_variants_get_apart(self):
        """只差标点的两个项目名不得得到同一个标签（#855 的现象）。"""
        self.assertNotEqual(exported_instance("wonton-chat-image"), exported_instance("wonton-chatimage"))

    def test_name_without_punctuation_unchanged(self):
        """没丢字符的标签保持原样，已有的栈标签不变。"""
        self.assertEqual(exported_instance("wonton-chatimage"), "chatimage")
        self.assertEqual(exported_instance("wonton-dev2"), "dev2")

    def test_truncated_name_gains_hash_suffix(self):
        """丢字符的标签是截短到 11 位的项目名加 5 位十六进制哈希，仍满足实例名规则。"""
        label = exported_instance("wonton-chat-image")
        self.assertRegex(label, r"^chatimage[0-9a-f]{5}$")

    def test_label_deterministic(self):
        self.assertEqual(exported_instance("wonton-chat-image"), exported_instance("wonton-chat-image"))

    def test_same_truncation_apart(self):
        """截短后相同的两个丢字符项目名靠哈希后缀区分。"""
        self.assertNotEqual(exported_instance("wonton-chatimage-extra"), exported_instance("wonton-chatimage-extray"))

    def test_long_name_without_punctuation_truncates(self):
        """超过 16 位且没丢字符的照旧截短到 16 位。"""
        self.assertEqual(exported_instance("wonton-abcdefghijklmnopq"), "abcdefghijklmnop")

    def test_all_punctuation_after_prefix(self):
        """去标点后什么都不剩时标签就是哈希，仍然合法。"""
        self.assertRegex(exported_instance("wonton-...-"), r"^[0-9a-f]{5}$")

    def test_default_project_has_no_instance(self):
        self.assertEqual(exported_instance("wonton-dev-core"), "")

    def test_explicit_instance_wins(self):
        self.assertEqual(exported_instance("wonton-chat-image", instance="mine"), "mine")

    def test_labels_stay_valid(self):
        for project in ["wonton-chat-image", "wonton-chatimage", "wonton-ChatImage", "wonton-...",
                        "wonton-a-b-c-d-e-f-g-h-i-j-k-l-m", "chat-image"]:
            with self.subTest(project=project):
                self.assertRegex(exported_instance(project), VALID_LABEL)


if __name__ == "__main__":
    unittest.main()
