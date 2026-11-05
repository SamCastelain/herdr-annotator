import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from install_support import install_paths, replace_helper


@unittest.skipIf(os.name == "nt", "Unix shell fixture; native installation is tested manually")
class InstallerTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        base = Path(self.temp.name)
        self.repo, self.home, self.app = base / "repo", base / "home", base / "app"
        (self.repo / "scripts").mkdir(parents=True)
        source = Path(__file__).resolve().parents[1]
        for name in ["install.py", "install_support.py", "tabs_integration.py", "update.py"]:
            shutil.copy2(source / "scripts" / name, self.repo / "scripts" / name)
        mod = self.repo / "mods/annotator"
        mod.mkdir(parents=True)
        for name in ["main.js", "review.js", "full.js", "syntax.js", "draft.js", "github-refresh.js", "updates.js", "launcher.js"]:
            (mod / name).write_text(f"// {name}\n")
        shutil.copy2(source / "mods/annotator/runtime.js", mod / "runtime.js")
        (mod / "mod.json").write_text(json.dumps({"version": "0.2.0"}))
        (self.repo / ".gitignore").write_text("target/\n__pycache__/\n")
        (self.repo / "source.txt").write_text("original")
        (self.repo / "target/release").mkdir(parents=True)
        (self.repo / "target/release/herdr-annotator").write_text("helper binary")
        (self.app / "mods/tabs").mkdir(parents=True)
        (self.app / "mods/tabs/main.js").write_text('.when(enabled.includes("changes"), el => el.child(panelButton("changes", "Changes", "mods/changes/changes.svg")))')
        (self.app / "layouts").mkdir()
        for name in ["one", "two"]:
            (self.app / "layouts" / f"{name}.json").write_text(json.dumps({"children": [
                {"content": {"mod": "tabs"}}, {"content": {"mod": "changes"}}]}))
        commands = base / "bin"
        commands.mkdir()
        for name, text in [("cargo", '#!/bin/sh\nif [ "$FAKE_EDIT_SOURCE" = "1" ]; then echo changed >> source.txt; fi\nexit 0\n'),
                           ("herdr-app", "#!/bin/sh\nexit 0\n")]:
            path = commands / name
            path.write_text(text)
            path.chmod(0o700)
        self.env = {**os.environ, "HOME": str(self.home), "HERDR_APP_HOME": str(self.app), "PATH": str(commands) + os.pathsep + os.environ["PATH"]}
        self.git("init", "-q")
        self.git("config", "user.name", "Test")
        self.git("config", "user.email", "test@example.invalid")
        self.git("add", ".")
        self.git("commit", "-qm", "source")

    def git(self, *args):
        return subprocess.run(["git", "-C", str(self.repo), *args], capture_output=True, text=True, check=True).stdout.strip()

    def install(self, *args, env=None):
        return subprocess.run([sys.executable, "scripts/install.py", *args], cwd=self.repo,
                              env=env or self.env, capture_output=True, text=True)

    def receipt(self):
        return json.loads((self.home / ".local/share/herdr-annotator/install.json").read_text())

    def test_install_remembers_all_layouts_and_updates_private_tabs_without_rewriting_layouts(self):
        first = self.install("--layout", "one", "--layout", "two")
        self.assertEqual(first.returncode, 0, first.stderr)
        receipt = self.receipt()
        self.assertEqual(receipt["layouts"], ["one", "two"])
        self.assertEqual(receipt["version"], "0.2.0")
        self.assertTrue(receipt["binary_digest"])
        self.assertTrue(receipt["updater_digest"])
        self.assertEqual(receipt["python"], sys.executable)
        self.assertEqual(receipt["runtime"]["helper"], receipt["binary"])
        self.assertTrue(receipt["runtime"]["updatesEnabled"])
        runtime = (self.app / "mods/annotator/runtime.js").read_text(encoding="utf-8")
        self.assertIn(json.dumps(sys.executable), runtime)
        self.assertEqual(runtime, (self.app / "mods/annotator-tabs/runtime.js").read_text(encoding="utf-8"))
        layouts = {path: path.read_bytes() for path in (self.app / "layouts").glob("*.json")}
        second = self.install()
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(self.receipt()["layouts"], ["one", "two"])
        self.assertEqual(layouts, {path: path.read_bytes() for path in layouts})
        self.assertTrue(list((self.home / ".local/share/herdr-annotator/backups").glob("*/install.json")))

    def test_modified_helper_and_updater_are_refused_before_replacing_mods(self):
        first = self.install()
        self.assertEqual(first.returncode, 0, first.stderr)
        for path in [self.home / ".local/bin/herdr-annotator", self.home / ".local/share/herdr-annotator/update.py", self.app / "mods/annotator/runtime.js"]:
            original = path.read_bytes()
            path.write_text("local edit")
            result = self.install()
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(path.read_text(), "local edit")
            path.write_bytes(original)

    def test_release_source_edit_during_build_prevents_all_install_writes(self):
        sha = self.git("rev-parse", "HEAD")
        result = self.install("--expected-commit", sha, env={**self.env, "FAKE_EDIT_SOURCE": "1"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("changed during the update", result.stderr)
        self.assertFalse((self.home / ".local/bin/herdr-annotator").exists())
        self.assertFalse((self.app / "mods/annotator").exists())
        self.assertIn("changed", (self.repo / "source.txt").read_text())

    def test_install_and_reinstall_with_unicode_spaces_and_shell_characters_in_paths(self):
        base = Path(self.temp.name)
        repo = base / "repo (中文) & 'quotes'"
        app = base / "app (中文) & 'quotes'"
        self.repo.rename(repo)
        self.app.rename(app)
        self.repo, self.app = repo, app
        self.home = base / "home (中文) & 'quotes'"
        self.env.update(HOME=str(self.home), HERDR_APP_HOME=str(self.app))
        first = self.install("--layout", "one")
        self.assertEqual(first.returncode, 0, first.stderr)
        receipt = self.receipt()
        self.assertEqual(receipt["repo"], str(self.repo))
        self.assertEqual(receipt["app_home"], str(self.app))
        runtime_path = self.app / "mods/annotator/runtime.js"
        declaration = next(line for line in runtime_path.read_text(encoding="utf-8").splitlines()
                           if line.startswith("export const installation = "))
        runtime = json.loads(declaration.removeprefix("export const installation = ").removesuffix(";"))
        self.assertEqual(runtime["helper"], str(self.home / ".local/bin/herdr-annotator"))
        self.assertEqual(runtime, receipt["runtime"])
        second = self.install()
        self.assertEqual(second.returncode, 0, second.stderr)

    def test_upgrade_adds_runtime_configuration_without_overwriting_unreceipted_files(self):
        first = self.install("--layout", "one")
        self.assertEqual(first.returncode, 0, first.stderr)
        receipt_path = self.home / ".local/share/herdr-annotator/install.json"
        receipt = self.receipt()
        for key, directory in [("files", self.app / "mods/annotator"), ("tabs_files", self.app / "mods/annotator-tabs")]:
            receipt[key].pop("runtime.js")
            (directory / "runtime.js").unlink()
        receipt_path.write_text(json.dumps(receipt), encoding="utf-8")
        unexpected = self.app / "mods/annotator-tabs/runtime.js"
        unexpected.write_text("local file", encoding="utf-8")
        refused = self.install()
        self.assertNotEqual(refused.returncode, 0)
        self.assertEqual(unexpected.read_text(encoding="utf-8"), "local file")
        unexpected.unlink()
        upgraded = self.install()
        self.assertEqual(upgraded.returncode, 0, upgraded.stderr)
        self.assertIn("runtime.js", self.receipt()["tabs_files"])


class InstallPlatformTest(unittest.TestCase):
    def test_windows_requires_explicit_app_home_and_preserves_exe_suffix(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            with self.assertRaisesRegex(ValueError, "HERDR_APP_HOME"):
                install_paths(home, {}, windows=True)
            app, binary, receipt = install_paths(home, {"HERDR_APP_HOME": str(home / "app")}, windows=True)
            self.assertEqual(app, (home / "app").resolve())
            self.assertEqual(binary.name, "herdr-annotator.exe")
            self.assertEqual(receipt, home / ".local/share/herdr-annotator/install.json")
            self.assertEqual(install_paths(home, {}, windows=False)[1].name, "herdr-annotator")

    def test_sharing_failure_is_bounded_and_preserves_old_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            source, binary = folder / "source.exe", folder / "herdr-annotator.exe"
            source.write_bytes(b"new")
            binary.write_bytes(b"old")
            with patch("install_support.os.replace", side_effect=PermissionError("sharing violation")) as replace, patch("install_support.time.sleep") as sleep:
                with self.assertRaisesRegex(PermissionError, "close running helper"):
                    replace_helper(source, binary, "test", windows=True)
            self.assertEqual(replace.call_count, 21)
            self.assertEqual(sleep.call_count, 20)
            self.assertEqual(binary.read_bytes(), b"old")
            self.assertEqual(sorted(p.name for p in folder.iterdir()), ["herdr-annotator.exe", "source.exe"])

    def test_successful_binary_replacement_cleans_temporary_file(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            source, binary = folder / "source.exe", folder / "herdr-annotator.exe"
            source.write_bytes(b"new")
            binary.write_bytes(b"old")
            replace_helper(source, binary, "test", windows=True)
            self.assertEqual(binary.read_bytes(), b"new")
            self.assertFalse((folder / ".herdr-annotator-test.exe").exists())


if __name__ == "__main__":
    unittest.main()
