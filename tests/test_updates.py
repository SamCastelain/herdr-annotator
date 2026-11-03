import fcntl
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import update


class UpdatesTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name).resolve()
        self.app = self.home / "app"
        self.updater = update.Updater(self.home, self.app)
        self.repo = self.home / "repo"
        self.repo.mkdir()
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True)
        self.git("config", "user.name", "Test")
        self.git("config", "user.email", "test@example.invalid")
        (self.repo / "source.txt").write_text("baseline")
        self.git("add", ".")
        self.git("commit", "-qm", "baseline")
        mod = self.app / "mods/annotator"
        mod.mkdir(parents=True)
        (mod / "main.js").write_text("installed mod")
        binary = self.home / ".local/bin/herdr-annotator"
        binary.parent.mkdir(parents=True)
        binary.write_text("installed helper")
        self.updater.directory.mkdir(parents=True)
        (self.updater.directory / "update.py").write_text("installed updater")
        self.receipt = {"repo": str(self.repo), "mod": str(mod), "binary": str(binary), "app_home": str(self.app),
                        "version": "0.2.0", "commit": self.git("rev-parse", "HEAD"),
                        "binary_digest": update.digest(binary), "updater_digest": update.digest(self.updater.directory / "update.py"),
                        "files": {"main.js": update.digest(mod / "main.js")}, "layouts": ["personal"]}
        update.atomic_json(self.updater.receipt_path, self.receipt)
        self.sha = "a" * 40
        self.latest = {"tag": "v0.3.0", "version": "0.3.0", "commit": self.sha}

    def git(self, *args):
        return subprocess.run(["git", "-C", str(self.repo), *args], check=True, capture_output=True, text=True).stdout.strip()

    def cache(self):
        update.atomic_json(self.updater.cache_path, {"checked_at": 100000, "installed": "0.2.0", "latest": self.latest})

    def test_daily_check_pins_commit_and_constructs_official_release_url(self):
        with patch.object(update, "api_json", side_effect=[{"tag_name": "v0.3.0", "html_url": "https://evil.invalid", "body": "Release notes"}, {"sha": self.sha}]) as api:
            result = self.updater.check(now=100000)
            self.assertEqual(result["latest"]["commit"], self.sha)
            self.assertEqual(result["latest"]["url"], f"https://github.com/{update.REPOSITORY}/releases/tag/v0.3.0")
            self.updater.check(now=100001)
            self.assertEqual(api.call_count, 2)

    def test_no_release_and_prerelease_do_not_offer_an_install(self):
        for release in [None, {"tag_name": "v0.3.0", "prerelease": True}, {"tag_name": "v0.1.0"}]:
            with patch.object(update, "api_json", return_value=release):
                self.assertIsNone(self.updater.check(force=True, now=100000)["latest"])

    def test_network_failure_keeps_previous_offer_and_backs_off(self):
        self.cache()
        with patch.object(update, "api_json", side_effect=OSError("offline")) as api:
            result = self.updater.check(force=True, now=200000)
            self.assertEqual(result["latest"], self.latest)
            self.assertIn("offline", result["error"])
            self.updater.check(now=200001)
            self.assertEqual(api.call_count, 1)

    def test_dirty_checkout_refuses_before_fetching_or_modifying_files(self):
        self.cache()
        (self.repo / "source.txt").write_text("user changes")
        with self.assertRaisesRegex(ValueError, "local changes"):
            self.updater.preflight("v0.3.0", self.sha)
        self.assertEqual((self.repo / "source.txt").read_text(), "user changes")
        self.assertEqual(json.loads(self.updater.receipt_path.read_text()), self.receipt)

    def test_modified_installed_files_are_preserved(self):
        self.cache()
        for path in [Path(self.receipt["mod"]) / "main.js", Path(self.receipt["binary"]), self.updater.directory / "update.py"]:
            original = path.read_text()
            path.write_text("local changes")
            with self.assertRaisesRegex(ValueError, "changed locally"):
                self.updater.preflight("v0.3.0", self.sha)
            self.assertEqual(path.read_text(), "local changes")
            path.write_text(original)

    def test_changed_release_tag_never_merges_or_installs(self):
        self.cache()
        real_git = self.updater.git
        def git(receipt, *args, **kwargs):
            if args[0] == "fetch":
                self.assertIn(update.UPSTREAM, args)
                return ""
            if args == ("rev-parse", "FETCH_HEAD^{commit}"):
                return "b" * 40
            return real_git(receipt, *args, **kwargs)
        with patch.object(self.updater, "git", side_effect=git):
            # Only perform() fetches. No worker/installer gets started here.
            with self.assertRaisesRegex(ValueError, "tag changed"):
                self.updater.perform("v0.3.0", self.sha)
        self.assertEqual(self.git("rev-parse", "HEAD"), self.receipt["commit"])

    def test_diverged_checkout_is_not_reset_or_merged(self):
        self.cache()
        calls = []
        def git(_receipt, *args, **kwargs):
            calls.append(args)
            if args[0] == "merge-base":
                raise ValueError("not an ancestor")
            if args == ("rev-parse", "--show-toplevel"):
                return str(self.repo)
            if args == ("rev-parse", "FETCH_HEAD^{commit}"):
                return self.sha
            if args[0] == "show":
                return json.dumps({"version": "0.3.0"})
            return ""
        with patch.object(self.updater, "git", side_effect=git):
            with self.assertRaisesRegex(ValueError, "diverged"):
                self.updater.perform("v0.3.0", self.sha)
        self.assertFalse(any(args[0] in {"merge", "reset", "checkout"} for args in calls))

    def test_confirmed_release_is_fast_forwarded_and_installed_with_pinned_commit(self):
        self.cache()
        calls = []
        def git(_receipt, *args, **kwargs):
            calls.append(args)
            if args == ("rev-parse", "--show-toplevel"):
                return str(self.repo)
            if args == ("rev-parse", "FETCH_HEAD^{commit}"):
                return self.sha
            if args[0] == "show":
                return json.dumps({"version": "0.3.0"})
            return ""
        def install(argv, **kwargs):
            self.assertEqual(argv, [sys.executable, "scripts/install.py", "--expected-commit", self.sha])
            self.assertEqual(kwargs["env"]["HERDR_APP_HOME"], str(self.app))
            update.atomic_json(self.updater.receipt_path, {**self.receipt, "version": "0.3.0", "commit": self.sha})
        with patch.object(self.updater, "git", side_effect=git), patch.object(update.subprocess, "run", side_effect=install):
            self.updater.perform("v0.3.0", self.sha)
        self.assertIn(("merge", "--ff-only", self.sha), calls)
        self.assertEqual(self.updater.receipt()["version"], "0.3.0")

    def test_worker_failure_records_error_and_keeps_installation_receipt(self):
        update.atomic_json(self.updater.job_path, {"id": "job", "status": "running"})
        with self.updater.lock_path.open("a") as lock, patch.object(self.updater, "perform", side_effect=ValueError("build failed")), patch("builtins.print"):
            self.updater.worker("v0.3.0", self.sha, "job", lock.fileno())
        self.assertEqual(self.updater.status()["status"], "failed")
        self.assertIn("build failed", self.updater.status()["error"])
        self.assertEqual(self.updater.receipt(), self.receipt)

    def test_detached_worker_runs_with_inherited_lock_and_reports_completion(self):
        self.cache()
        script = self.updater.directory / "update.py"
        script.write_text("""import json,os,pathlib,sys
os.fstat(int(sys.argv[sys.argv.index('--lock-fd')+1]))
path=pathlib.Path(__file__).with_name('update-job.json')
job=json.loads(path.read_text())
job['status']='success'
temp=path.with_suffix('.done')
print('fake-worker-complete',flush=True)
temp.write_text(json.dumps(job))
os.replace(temp,path)
""")
        update.atomic_json(self.updater.receipt_path, {**self.receipt, "updater_digest": update.digest(script)})
        self.updater.start("v0.3.0", self.sha)
        deadline = time.monotonic() + 5
        while self.updater.status().get("status") == "running" and time.monotonic() < deadline:
            time.sleep(0.02)
        self.assertEqual(self.updater.status()["status"], "success")
        self.assertIn("fake-worker-complete", self.updater.log_path.read_text())

    def test_background_launch_does_not_inherit_herdr_output_pipes(self):
        self.cache()
        with patch.object(self.updater, "preflight", return_value=self.receipt), patch.object(update.subprocess, "Popen") as spawn:
            job = self.updater.start("v0.3.0", self.sha)
        self.assertEqual(job["status"], "running")
        options = spawn.call_args.kwargs
        self.assertTrue(options["start_new_session"])
        self.assertEqual(len(options["pass_fds"]), 1)
        self.assertEqual(options["stdin"], subprocess.DEVNULL)
        self.assertIsNotNone(options["stdout"])
        self.assertEqual(self.updater.status()["status"], "failed", "a dead worker cannot leave the UI locked forever")

    def test_lock_reports_live_worker_and_rejects_duplicate_launch(self):
        self.cache()
        update.atomic_json(self.updater.job_path, {"id": "j1", "status": "running"})
        with self.updater.lock_path.open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertEqual(self.updater.status()["status"], "running")
            with self.assertRaisesRegex(ValueError, "already running"):
                self.updater.start("v0.3.0", self.sha)


if __name__ == "__main__":
    unittest.main()
