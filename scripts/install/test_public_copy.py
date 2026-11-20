"""公开副本的检查和同步：假 api.github.com、本地裸仓当公开远端，不用网络和凭据。

check-public-copy.sh 的 fixture 用 `git clone --depth 2` 造，和 CI 的检出一样只带两个提交：
第二行记的提交在本地取不到，检查只能靠 API。
"""

import http.server
import os
import shutil
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
MAIN_PATH = "deploy/install/install.sh"
SCRIPTS = ("check-public-copy.sh", "sync-public.sh")

GIT_ENV = {
    "GIT_AUTHOR_NAME": "t",
    "GIT_AUTHOR_EMAIL": "t@example.invalid",
    "GIT_COMMITTER_NAME": "t",
    "GIT_COMMITTER_EMAIL": "t@example.invalid",
}


def git(*args, cwd):
    return subprocess.run(
        ["git", *args],
        cwd=cwd,
        env=os.environ | GIT_ENV,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


class FakeApi:
    """按路径回话的假 GitHub API；每个测试改 state。"""

    def __init__(self):
        self.pin_status = "ahead"
        self.commit_exists = True
        self.limited = False
        self.public = b""
        self.source = b""
        api = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                if api.limited:
                    return self.reply(403, b'{"message": "API rate limit exceeded"}')
                if "/compare/" in self.path:
                    return self.reply(200, ('{"status": "%s"}' % api.pin_status).encode())
                if "/wonton-install/contents/install.sh" in self.path:
                    return self.reply(200, api.public)
                if "/wonton/contents/" in self.path:
                    return self.reply(200, api.source)
                if "/wonton/commits/" in self.path:
                    return self.reply(200 if api.commit_exists else 404, b"{}")
                return self.reply(404, b"{}")

            def reply(self, code, body):
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = "http://127.0.0.1:%d" % self.server.server_port
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def stop(self):
        self.server.shutdown()
        self.server.server_close()


class Fixture(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp)
        self.api = FakeApi()
        self.addCleanup(self.api.stop)
        self.repo = self.main_repo()

    def main_repo(self):
        """主仓库的历史按 documenthub 建好再 shallow clone，克隆里只有一个提交本地可取。"""
        origin = self.tmp / "wonton"
        origin.mkdir()
        git("init", "-q", "-b", "main", cwd=origin)
        (origin / "scripts/install").mkdir(parents=True)
        for name in SCRIPTS:
            shutil.copy(HERE / name, origin / "scripts/install" / name)
        write(origin / MAIN_PATH, "v1\n")
        write(origin / "deploy/install/public-copy", "1" * 40 + "\n" + "2" * 40 + "\n")
        git("add", "-A", cwd=origin)
        git("commit", "-q", "-m", "v1", cwd=origin)
        self.oldest = git("rev-parse", "HEAD", cwd=origin)
        for version in ("v2", "v3"):
            write(origin / MAIN_PATH, version + "\n")
            git("commit", "-q", "-am", version, cwd=origin)
        repo = self.tmp / "repo"
        git("clone", "-q", "--depth", "2", "file://" + str(origin), str(repo), cwd=self.tmp)
        git("config", "user.name", "t", cwd=repo)
        git("config", "user.email", "t@example.invalid", cwd=repo)
        return repo

    def pin_file(self, pin, source, blob=None):
        if blob is None:
            blob = git("hash-object", MAIN_PATH, cwd=self.repo)
        lines = [pin, source] + ([blob] if blob else [])
        write(self.repo / "deploy/install/public-copy", "\n".join(lines) + "\n")

    def matching(self):
        """API 上的内容和提交都和工作树对得上，pin 是 main 上的提交。"""
        content = (self.repo / MAIN_PATH).read_bytes()
        self.api.public = content
        self.api.source = content
        self.pin_file("1" * 40, git("rev-parse", "HEAD", cwd=self.repo))

    def check(self, token=True):
        env = os.environ | {"WONTON_GITHUB_API": self.api.url}
        env["GH_TOKEN"] = "t0ken" if token else ""
        return subprocess.run(
            ["bash", str(self.repo / "scripts/install/check-public-copy.sh")],
            cwd=self.repo,
            env=env,
            capture_output=True,
            text=True,
        )


class CheckTest(Fixture):
    def test_a_source_commit_outside_the_shallow_history_passes(self):
        self.matching()
        # 和 CI 一样：第二行记的提交本地没有，只有 API 能读到。
        self.pin_file("1" * 40, self.oldest)
        missing = subprocess.run(
            ["git", "cat-file", "-e", self.oldest], cwd=self.repo, capture_output=True
        )
        self.assertNotEqual(missing.returncode, 0)
        done = self.check()
        self.assertEqual(done.returncode, 0, done.stderr)

    def test_a_changed_install_sh_without_a_new_blob_is_red(self):
        self.matching()
        write(self.repo / MAIN_PATH, "v4\n")
        done = self.check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("sync-public.sh", done.stderr)

    def test_a_public_copy_with_two_lines_says_to_add_the_third(self):
        self.matching()
        write(self.repo / "deploy/install/public-copy", "1" * 40 + "\n" + "2" * 40 + "\n")
        done = self.check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("第三行", done.stderr)
        self.assertIn("hash-object", done.stderr)

    def test_a_missing_source_commit_says_it_is_missing(self):
        self.matching()
        self.api.commit_exists = False
        done = self.check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("找不到", done.stderr)

    def test_rate_limiting_is_not_reported_as_a_missing_commit(self):
        self.matching()
        self.api.limited = True
        done = self.check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("403", done.stderr)
        self.assertNotIn("找不到", done.stderr)

    def test_a_pin_that_is_not_on_public_main_is_red(self):
        self.matching()
        self.api.pin_status = "behind"
        done = self.check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("不在公开仓库 main 上", done.stderr)

    def test_public_content_that_differs_is_red_with_a_diff(self):
        self.matching()
        self.api.public = b"v1\n"
        done = self.check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("-v1", done.stderr)
        self.assertIn("+v3", done.stderr)


class SyncTest(Fixture):
    def setUp(self):
        super().setUp()
        # sync 只认 origin/main 的对照，root 的 origin 指向主仓库的裸仓。
        self.origin = self.tmp / "wonton-origin.git"
        git("clone", "-q", "--bare", str(self.tmp / "wonton"), str(self.origin), cwd=self.tmp)
        git("remote", "set-url", "origin", str(self.origin), cwd=self.repo)
        git("fetch", "-q", "origin", "main", cwd=self.repo)
        git("switch", "-q", "-c", "fix/x", cwd=self.repo)
        # 公开仓库是独立的历史，只有一份老的 install.sh。
        self.public = self.tmp / "wonton-install.git"
        git("init", "-q", "--bare", "-b", "main", str(self.public), cwd=self.tmp)
        self.clone = self.tmp / "wonton-install"
        git("clone", "-q", str(self.public), str(self.clone), cwd=self.tmp)
        git("config", "user.name", "t", cwd=self.clone)
        git("config", "user.email", "t@example.invalid", cwd=self.clone)
        write(self.clone / "install.sh", "v3\n")
        git("add", "-A", cwd=self.clone)
        git("commit", "-q", "-m", "老副本", cwd=self.clone)
        git("push", "-q", "origin", "HEAD:main", cwd=self.clone)

    def sync(self):
        return subprocess.run(
            ["bash", str(self.repo / "scripts/install/sync-public.sh"), str(self.clone)],
            cwd=self.repo,
            env=os.environ | GIT_ENV,
            capture_output=True,
            text=True,
        )

    def lines(self):
        return (self.repo / "deploy/install/public-copy").read_text().splitlines()

    def test_a_branch_commit_is_recorded_and_the_result_is_committable(self):
        write(self.repo / MAIN_PATH, "v4\n")
        git("commit", "-q", "-am", "v4", cwd=self.repo)
        head = git("rev-parse", "HEAD", cwd=self.repo)
        done = self.sync()
        self.assertEqual(done.returncode, 0)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(self.lines()[1], head)
        self.assertEqual(self.lines()[2], git("hash-object", MAIN_PATH, cwd=self.repo))
        self.assertIn("public-copy", git("status", "--porcelain", cwd=self.repo))
        self.assertEqual(
            (self.clone / "install.sh").read_bytes(), (self.repo / MAIN_PATH).read_bytes()
        )
        self.assertTrue(git("branch", "--list", "sync/*", cwd=self.clone))

    def test_sync_refuses_an_uncommitted_install_sh(self):
        before = (self.repo / "deploy/install/public-copy").read_bytes()
        write(self.repo / MAIN_PATH, "v4\n")
        done = self.sync()
        self.assertEqual(done.returncode, 1)
        self.assertIn("先提交", done.stderr)
        self.assertEqual((self.repo / "deploy/install/public-copy").read_bytes(), before)

    def test_after_the_public_merge_a_second_run_pins_main_and_the_check_goes_green(self):
        write(self.repo / MAIN_PATH, "v4\n")
        git("commit", "-q", "-am", "v4", cwd=self.repo)
        self.assertEqual(self.sync().returncode, 0)
        # 公开 PR 合了：同步分支快进进 main，克隆换回 main。
        git("push", "-q", "origin", "HEAD:main", cwd=self.clone)
        git("switch", "-q", "main", cwd=self.clone)
        git("pull", "-q", "--ff-only", "origin", "main", cwd=self.clone)
        merged = git("rev-parse", "HEAD", cwd=self.clone)
        done = self.sync()
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(self.lines()[0], merged)
        self.assertEqual(len(self.lines()), 3)
        self.api.public = (self.repo / MAIN_PATH).read_bytes()
        # 主仓库的 PR 这时才能绿：公开 main 的内容、检查记的提交都和这份一致。
        self.api.source = (self.repo / MAIN_PATH).read_bytes()
        self.api.pin_status = "ahead"
        done = self.check()
        self.assertEqual(done.returncode, 0)


class ReadmeTest(unittest.TestCase):
    def test_the_readme_describes_the_third_line(self):
        text = (ROOT / "deploy/install/README.md").read_text()
        self.assertIn("三行", text)
        self.assertIn("git hash-object", text)
        self.assertIn("check-public-copy.sh", text)
