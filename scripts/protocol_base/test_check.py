"""check-protocol-base.sh 的行为：没改协议直接过；改了协议、main 在分支之后又改过协议就挡；main 只改了别的照样过。
在临时目录里建一个裸仓库当 origin，分支和 main 的提交都是假的。"""
import os
import subprocess
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "check-protocol-base.sh")
ENV = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@example.com", GIT_COMMITTER_NAME="t",
           GIT_COMMITTER_EMAIL="t@example.com", GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_SYSTEM=os.devnull)


class ProtocolBase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.origin = os.path.join(self.tmp.name, "origin.git")
        self.work = os.path.join(self.tmp.name, "work")
        self.git(self.tmp.name, "init", "--quiet", "--bare", "-b", "main", self.origin)
        self.git(self.tmp.name, "clone", "--quiet", self.origin, self.work)
        self.commit("crates/wonton-protocol/src/lib.rs", "v1", "init")
        self.git(self.work, "push", "--quiet", "origin", "HEAD:main")

    def tearDown(self):
        self.tmp.cleanup()

    def git(self, cwd, *args):
        return subprocess.run(["git", *args], cwd=cwd, env=ENV, check=True, capture_output=True, text=True).stdout.strip()

    def commit(self, path, text, msg):
        full = os.path.join(self.work, path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w") as f:
            f.write(text)
        self.git(self.work, "add", path)
        self.git(self.work, "commit", "--quiet", "-m", msg)
        return self.git(self.work, "rev-parse", "HEAD")

    def branch(self, path):
        """从当前 main 切分支改 path，推上去，回到 main；返回分支头。"""
        self.git(self.work, "checkout", "--quiet", "-b", f"b{len(path)}", "origin/main")
        head = self.commit(path, "branch", "branch change")
        self.git(self.work, "push", "--quiet", "origin", f"HEAD:refs/heads/b{len(path)}")
        self.git(self.work, "checkout", "--quiet", "--detach", "origin/main")
        return head

    def main_moves(self, path):
        self.git(self.work, "fetch", "--quiet", "origin")
        self.git(self.work, "checkout", "--quiet", "--detach", "origin/main")
        self.commit(path, "main", "main change")
        self.git(self.work, "push", "--quiet", "origin", "HEAD:main")

    def check(self, head):
        return subprocess.run(["bash", SCRIPT, "main", head], cwd=self.work, env=ENV, capture_output=True, text=True)

    def test_untouched_protocol_passes_even_if_main_moved_it(self):
        head = self.branch("apps/ios/a.swift")
        self.main_moves("crates/wonton-protocol/src/lib.rs")
        self.assertEqual(self.check(head).returncode, 0)

    def test_protocol_branch_behind_protocol_change_fails(self):
        head = self.branch("crates/wonton-protocol/src/new.rs")
        self.main_moves("crates/wonton-protocol/src/lib.rs")
        r = self.check(head)
        self.assertEqual(r.returncode, 1)
        self.assertIn("先同步 main", r.stdout)

    def test_protocol_branch_with_unrelated_main_change_passes(self):
        head = self.branch("crates/wonton-protocol/src/new.rs")
        self.main_moves("apps/ios/b.swift")
        self.assertEqual(self.check(head).returncode, 0)


if __name__ == "__main__":
    unittest.main()
