#!/usr/bin/env python3
"""Check official releases; install only through an explicit, detached update job."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import uuid

REPOSITORY = "jakub-tucek/herdr-annotator"
UPSTREAM = f"https://github.com/{REPOSITORY}.git"
API = f"https://api.github.com/repos/{REPOSITORY}"
DAY = 86400
TAG = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\Z")
SHA = re.compile(r"[0-9a-f]{40}\Z")


def version(value):
    match = TAG.fullmatch("v" + str(value).removeprefix("v"))
    if not match:
        raise ValueError("Only stable MAJOR.MINOR.PATCH releases are supported")
    return tuple(map(int, match.groups()))


def read_json(path, default=None):
    if not path.exists():
        return default
    if path.stat().st_size > 1024 * 1024:
        raise ValueError(f"State file too large; not overwritten: {path}")
    return json.loads(path.read_text())


def atomic_json(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(f".{path.name}-{uuid.uuid4().hex}")
    try:
        with open(temp, "x", opener=lambda path, flags: os.open(path, flags, 0o600)) as stream:
            json.dump(data, stream)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp, path)
    finally:
        temp.unlink(missing_ok=True)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def api_json(endpoint):
    request = urllib.request.Request(API + endpoint, headers={
        "User-Agent": "herdr-annotator-updater", "Accept": "application/vnd.github+json",
    })
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            content = response.read(1024 * 1024 + 1)
            if len(content) > 1024 * 1024:
                raise ValueError("GitHub response exceeds 1 MiB")
            return json.loads(content)
    except urllib.error.HTTPError as error:
        if error.code == 404 and endpoint == "/releases/latest":
            return None  # No published stable release yet.
        raise


class Updater:
    def __init__(self, home=None, app_home=None):
        self.home = Path(home or Path.home()).resolve()
        self.directory = self.home / ".local/share/herdr-annotator"
        self.receipt_path = self.directory / "install.json"
        self.cache_path = self.directory / "update-check.json"
        self.job_path = self.directory / "update-job.json"
        self.lock_path = self.directory / "update.lock"
        self.log_path = self.directory / "update.log"
        self.app_home = Path(app_home or os.environ.get("HERDR_APP_HOME", self.home / ".config/herdr-app")).resolve()

    def receipt(self):
        receipt = read_json(self.receipt_path)
        if not receipt or not receipt.get("version") or not receipt.get("binary_digest") or not receipt.get("updater_digest"):
            raise ValueError("Run the current scripts/install.py once to enable update prompts")
        if Path(receipt.get("app_home", "")).resolve() != self.app_home:
            raise ValueError("Installation belongs to a different Herdr app home")
        version(receipt["version"])
        return receipt

    def check(self, force=False, now=None):
        receipt = self.receipt()
        now = time.time() if now is None else now
        cache = read_json(self.cache_path, {})
        if force or now - cache.get("checked_at", 0) >= DAY or cache.get("installed") != receipt["version"]:
            try:
                release = api_json("/releases/latest")
                latest = None
                if release and not release.get("draft") and not release.get("prerelease"):
                    tag = release.get("tag_name", "")
                    if not TAG.fullmatch(tag):
                        raise ValueError("Release tag must be vMAJOR.MINOR.PATCH")
                    if version(tag) > version(receipt["version"]):
                        commit = api_json(f"/commits/{tag}").get("sha", "")
                        if not SHA.fullmatch(commit):
                            raise ValueError("GitHub returned an invalid release commit")
                        latest = {"tag": tag, "version": tag[1:], "commit": commit,
                                  "url": f"https://github.com/{REPOSITORY}/releases/tag/{tag}",
                                  "notes": str(release.get("body") or "")[:16000]}
                cache = {"checked_at": now, "installed": receipt["version"], "latest": latest, "error": None}
            except (OSError, ValueError, TypeError, AttributeError) as error:
                cache = {**cache, "checked_at": now, "installed": receipt["version"], "error": str(error)[:1000]}
            atomic_json(self.cache_path, cache)
        latest = cache.get("latest")
        if latest and version(latest["version"]) <= version(receipt["version"]):
            latest = None
        return {**cache, "latest": latest, "current": receipt["version"], "job": self.status()}

    def git(self, receipt, *args, timeout=10):
        env = {**os.environ, "GIT_TERMINAL_PROMPT": "0"}
        result = subprocess.run(["git", "-C", receipt["repo"], "-c", "core.hooksPath=/dev/null", *args],
                                capture_output=True, text=True, timeout=timeout, env=env)
        if result.returncode:
            raise ValueError((result.stderr or result.stdout or "Git command failed").strip()[-2000:])
        return result.stdout.strip()

    def verify_files(self, receipt):
        binary = Path(receipt["binary"])
        if not binary.is_file() or digest(binary) != receipt["binary_digest"]:
            raise ValueError("Installed helper changed locally; update refused")
        updater = self.directory / "update.py"
        if not updater.is_file() or digest(updater) != receipt["updater_digest"]:
            raise ValueError("Installed updater changed locally; update refused")
        for folder, key in [(Path(receipt["mod"]), "files"), (self.app_home / "mods/annotator-tabs", "tabs_files")]:
            if folder.is_symlink():
                raise ValueError(f"Linked mod requires a manual update: {folder}")
            for name, checksum in receipt.get(key, {}).items():
                if Path(name).name != name:
                    raise ValueError("Invalid install receipt filename")
                path = folder / name
                if not path.is_file() or digest(path) != checksum:
                    raise ValueError(f"Installed mod changed locally; update refused: {path}")

    def preflight(self, tag, commit):
        if not TAG.fullmatch(tag) or not SHA.fullmatch(commit):
            raise ValueError("Invalid release identity")
        receipt = self.receipt()
        latest = read_json(self.cache_path, {}).get("latest")
        if not latest or (latest.get("tag"), latest.get("commit")) != (tag, commit):
            raise ValueError("Check for updates again before installing this release")
        if version(tag) <= version(receipt["version"]):
            raise ValueError("This release is already installed or older")
        self.verify_files(receipt)
        if Path(self.git(receipt, "rev-parse", "--show-toplevel")).resolve() != Path(receipt["repo"]).resolve():
            raise ValueError("Install source is not the original repository root")
        if self.git(receipt, "status", "--porcelain=v1", "--untracked-files=all"):
            raise ValueError("Source checkout has local changes. Commit/stash them yourself, or update manually")
        return receipt

    def perform(self, tag, commit, lock_fd=None):
        receipt = self.preflight(tag, commit)
        self.git(receipt, "fetch", "--no-tags", "--no-auto-maintenance", UPSTREAM, f"refs/tags/{tag}", timeout=120)
        fetched = self.git(receipt, "rev-parse", "FETCH_HEAD^{commit}")
        if fetched != commit:
            raise ValueError("Release tag changed since the check; refusing to install it")
        manifest = json.loads(self.git(receipt, "show", f"{commit}:mods/annotator/mod.json"))
        if manifest.get("version") != tag[1:]:
            raise ValueError("Release tag and mod version do not match; refusing to install")
        # An unrelated local commit must never be merged, reset or discarded.
        try:
            self.git(receipt, "merge-base", "--is-ancestor", "HEAD", commit)
        except ValueError as error:
            raise ValueError("Source checkout has diverged from this release; update manually without discarding your commits") from error
        self.preflight(tag, commit)
        self.git(receipt, "merge", "--ff-only", commit)
        env = {**os.environ, "HERDR_APP_HOME": str(self.app_home)}
        subprocess.run([sys.executable, "scripts/install.py", "--expected-commit", commit],
                       cwd=receipt["repo"], env=env, check=True, timeout=1800,
                       pass_fds=() if lock_fd is None else (lock_fd,))
        installed = self.receipt()
        if installed["version"] != tag[1:] or installed.get("commit") != commit:
            raise ValueError("Installer did not record the expected release")

    def status(self):
        state = read_json(self.job_path, {})
        if state.get("status") == "running":
            self.directory.mkdir(parents=True, exist_ok=True)
            with self.lock_path.open("a") as lock:
                try:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError:
                    return state
                state = read_json(self.job_path, {})
                if state.get("status") == "running":
                    state = {**state, "status": "failed", "error": "Update worker stopped. Inspect update.log, then retry or run scripts/install.py manually"}
                    atomic_json(self.job_path, state)
        return state

    def start(self, tag, commit):
        self.preflight(tag, commit)
        self.directory.mkdir(parents=True, exist_ok=True)
        with self.lock_path.open("a") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise ValueError("An update is already running") from error
            job = {"id": uuid.uuid4().hex, "status": "running", "tag": tag, "commit": commit, "started_at": time.time()}
            atomic_json(self.job_path, job)
            with self.log_path.open("w") as log:
                try:
                    process = subprocess.Popen([sys.executable, str(self.directory / "update.py"), "worker", "--app-home", str(self.app_home),
                                      "--tag", tag, "--commit", commit, "--lock-fd", str(lock.fileno()), "--job", job["id"]],
                                     stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True,
                                     pass_fds=(lock.fileno(),))
                    # Reap if this caller stays alive (e.g. tests); a short CLI
                    # exits without waiting and the detached worker survives.
                    threading.Thread(target=process.wait, daemon=True).start()
                except OSError as error:
                    atomic_json(self.job_path, {**job, "status": "failed", "error": str(error)})
                    raise
            return job

    def worker(self, tag, commit, job_id, lock_fd):
        # Retain the descriptor inherited from start(), including during install.
        os.fstat(lock_fd)
        state = read_json(self.job_path, {})
        if state.get("id") != job_id or state.get("status") != "running":
            raise ValueError("Update job no longer owns the installation")
        try:
            self.perform(tag, commit, lock_fd)
            state = {**state, "status": "success", "error": None}
        except Exception as error:
            print(str(error), flush=True)
            state = {**state, "status": "failed", "error": str(error)[-2000:] + f". Log: {self.log_path}"}
        atomic_json(self.job_path, state)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["check", "start", "status", "worker"])
    parser.add_argument("--app-home")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--tag", default="")
    parser.add_argument("--commit", default="")
    parser.add_argument("--job")
    parser.add_argument("--lock-fd", type=int)
    args = parser.parse_args()
    try:
        updater = Updater(app_home=args.app_home)
        if args.command == "check":
            data = updater.check(args.force)
        elif args.command == "start":
            data = updater.start(args.tag, args.commit)
        elif args.command == "status":
            data = updater.status()
        else:
            if args.lock_fd is None:
                raise ValueError("Worker requires an inherited installation lock")
            updater.worker(args.tag, args.commit, args.job, args.lock_fd)
            return 0
        print(json.dumps({"ok": True, "data": data}))
        return 0
    except Exception as error:
        print(json.dumps({"ok": False, "error": str(error)}))
        return 1


if __name__ == "__main__":
    sys.exit(main())
