"""nightly-ui-report.py 的 Android 解析：JUnit 的 assumption 跳过不算失败。"""

import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("nightly-ui-report.py")
spec = importlib.util.spec_from_file_location("nightly_ui_report", SCRIPT)
report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(report)


def parse(xml: str) -> list[str]:
    with tempfile.TemporaryDirectory() as results_dir:
        Path(results_dir, "TEST-app.WontonTest.xml").write_text(xml, encoding="utf-8")
        return report.android_failures(results_dir)


class AndroidAssumptionTest(unittest.TestCase):
    def test_type_attribute_marks_assumption_as_skipped(self):
        self.assertEqual(
            parse(
                """<testsuite name="app.WontonTest" tests="1">
  <testcase classname="app.WontonTest" name="needsStack">
    <failure type="org.junit.AssumptionViolatedException" message="需要 -e HISTORY_TEXT_B64">at org.junit.Assume</failure>
  </testcase>
</testsuite>"""
            ),
            [],
        )

    def test_message_attribute_marks_assumption_as_skipped(self):
        self.assertEqual(
            parse(
                """<testsuite name="app.WontonTest" tests="1">
  <testcase classname="app.WontonTest" name="needsStack">
    <failure message="org.junit.AssumptionViolatedException: 需要 -e HISTORY_TEXT_B64">at org.junit.Assume</failure>
  </testcase>
</testsuite>"""
            ),
            [],
        )

    def test_exception_in_element_text_marks_assumption_as_skipped(self):
        # AGP 的真实写法和这里一样：type/message 都没有，异常类在正文第一行。
        self.assertEqual(
            parse(
                """<testsuite name="app.WontonTest" tests="1">
  <testcase classname="app.WontonTest" name="needsStack">
    <failure>org.junit.AssumptionViolatedException: 需要 -e HISTORY_TEXT_B64
\tat org.junit.Assume.assumeTrue(Assume.java:68)
</failure>
  </testcase>
</testsuite>"""
            ),
            [],
        )

    def test_junit5_test_aborted_is_skipped(self):
        self.assertEqual(
            parse(
                """<testsuite name="app.WontonTest" tests="1">
  <testcase classname="app.WontonTest" name="needsStack">
    <failure type="org.opentest4j.TestAbortedException" message="需要开发栈"/>
  </testcase>
</testsuite>"""
            ),
            [],
        )

    def test_subclass_type_is_skipped(self):
        self.assertEqual(
            parse(
                """<testsuite name="app.WontonTest" tests="1">
  <testcase classname="app.WontonTest" name="needsStack">
    <failure type="org.junit.internal.AssumptionViolatedException" message="跳过"/>
  </testcase>
</testsuite>"""
            ),
            [],
        )

    def test_real_failure_is_still_reported(self):
        self.assertEqual(
            parse(
                """<testsuite name="app.WontonTest" tests="1">
  <testcase classname="app.WontonTest" name="broken">
    <failure type="java.lang.AssertionError" message="expected true">at app.WontonTest</failure>
  </testcase>
</testsuite>"""
            ),
            ["WontonTest.broken"],
        )

    def test_real_error_is_still_reported(self):
        self.assertEqual(
            parse(
                """<testsuite name="app.WontonTest" tests="1">
  <testcase classname="app.WontonTest" name="crashed">
    <error type="java.lang.IllegalStateException" message="boom"/>
  </testcase>
</testsuite>"""
            ),
            ["WontonTest.crashed"],
        )

    def test_assumption_and_real_failure_in_one_suite(self):
        self.assertEqual(
            parse(
                """<testsuite name="app.WontonTest" tests="2">
  <testcase classname="app.WontonTest" name="needsStack">
    <failure type="org.junit.AssumptionViolatedException" message="需要 -e HISTORY_TEXT_B64"/>
  </testcase>
  <testcase classname="app.WontonTest" name="broken">
    <failure type="java.lang.AssertionError" message="expected true"/>
  </testcase>
</testsuite>"""
            ),
            ["WontonTest.broken"],
        )


if __name__ == "__main__":
    unittest.main()
