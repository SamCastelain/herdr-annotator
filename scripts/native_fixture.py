#!/usr/bin/env python3
"""Create an isolated review fixture, optionally open it in Herdr App. Never sends."""
import argparse
import json
from pathlib import Path
import shlex
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--parent", help="Parent directory for temporary fixture artifacts")
parser.add_argument("--open", action="store_true", help="Open one preview mod tab in the current space")
parser.add_argument("--states", action="store_true", help="Include an outdated note and a wrapped code line")
parser.add_argument("--full", action="store_true", help="Show the standalone full review")
parser.add_argument("--offline", action="store_true", help="Simulate an unreachable helper without contacting a remote machine")
args = parser.parse_args()
root = Path(tempfile.mkdtemp(prefix="herdr-annotator-native-", dir=args.parent))
repo = root / "checkout"
repo.mkdir()

def git(*argv):
    subprocess.run(["git", "-C", str(repo), *argv], check=True, capture_output=True)

git("init", "--initial-branch=main")
git("config", "user.name", "Annotator Fixture")
git("config", "user.email", "fixture@example.invalid")
source = repo / "payment.rs"
source.write_text('pub fn fee(amount: u64) -> u64 {\n    amount / 10\n}\n')
git("add", ".")
git("commit", "-m", "fixture baseline")
git("branch", "develop")
source.write_text('pub fn fee(amount: u64) -> u64 {\n    amount / 100\n}\n')
(repo / "README.md").write_text("# Review fixture\n\nThis is disposable test data, not a real project.\n")
binary = Path(__file__).resolve().parents[1] / "target/release/herdr-annotator"
helper = root / "helper"
helper.write_text(f'#!/bin/sh\nexport HERDR_ANNOTATOR_STATE_DIR={shlex.quote(str(root / "state"))}\nexec {shlex.quote(str(binary))} "$@"\n')
helper.chmod(0o700)

def request(value):
    value["scope"] = {"repo": str(repo), "machine": None}
    value["view"] = {"file": "payment.rs", "context": 3}
    reply = json.loads(subprocess.check_output([str(helper), "request", json.dumps(value)], text=True))
    assert reply["ok"], reply
    return reply["data"]

snapshot = request({"op": "snapshot"})
request({"op": "add", "revision": snapshot["snapshot"]["revision"], "path": "payment.rs",
         "selection": {"side": "new", "start": 2, "end": 2},
         "body": "Check rounding here. Should a fractional fee round up?"})
if args.states:
    request({"op": "add", "revision": snapshot["snapshot"]["revision"], "path": "payment.rs",
             "selection": {"side": "new", "start": 1, "end": 1},
             "body": "Keep the public function name consistent with the other fee helpers."})
    source.write_text('pub fn calculate_fee(amount: u64) -> u64 {\n'
                      '    // Round fractional fees deliberately; callers rely on predictable results for very small payments.\n'
                      '    amount / 100\n}\n')
result = {"fixture": str(root), "scope": {"repo": str(repo), "machine": None}, "helper": str(helper), "file": "payment.rs"}
if args.full:
    result["presentation"] = "full"
if args.offline:
    error = {"ok": False, "error": "Command failed (5): " + json.dumps({"error": {
        "kind": "unavailable", "message": "offline-fixture is unreachable: SSH connection timed out"}})}
    helper.write_text("#!/usr/bin/env python3\nimport json\nprint(" + repr(json.dumps(error)) + ")\nraise SystemExit(1)\n")
    result["scope"]["machine"] = "offline-fixture"
if args.open:
    status = json.loads(subprocess.check_output(["herdr-app", "status"], text=True))
    command = {"name": "Annotator preview", "space": status["space"], "mod": "annotator", "props": result}
    result["tab_result"] = json.loads(subprocess.check_output(["herdr-app", "do", "tab.new", json.dumps(command)], text=True))
    result["previous_tab"] = status["tab"]
print(json.dumps(result, indent=2))
