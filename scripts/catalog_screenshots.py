#!/usr/bin/env python3
"""Capture native catalog/README images from inert demo data (macOS, sips)."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", required=True, help="Directory for native README screenshots")
args = parser.parse_args()
if not shutil.which("sips"):
    parser.error("This capture script requires macOS sips for cropping")
output = Path(args.output).resolve()
output.mkdir(parents=True, exist_ok=True)
root = Path(__file__).resolve().parents[1]
source = root / "mods/annotator"
app_home = Path(os.environ.get("HERDR_APP_HOME", Path.home() / ".config/herdr-app"))
drivers = []
scope = {"repo": "/workspace/payments", "machine": None, "comparison": {"mode": "branch", "base": "main"}}
rows = [{"kind": "hunk", "old": None, "new": None, "text": "@@ -1,8 +1,13 @@"}]
lines = [
    ("context", 1, 1, "export function feeFor(cents: number): number {"),
    ("delete", 2, None, "  return Math.round(cents * 0.025);"),
    ("add", None, 2, "  if (!Number.isSafeInteger(cents) || cents < 0) {"),
    ("add", None, 3, '    throw new RangeError("Expected non-negative cents");'),
    ("add", None, 4, "  }"),
    ("add", None, 5, ""),
    ("add", None, 6, "  const feeCents = Math.ceil((cents * 25) / 1000);"),
    ("add", None, 7, "  return Math.min(feeCents, 2500);"),
    ("context", 3, 8, "}"),
    ("context", 4, 9, ""),
    ("context", 5, 10, "export function netAmount(cents: number): number {"),
    ("context", 6, 11, "  const fee = feeFor(cents);"),
    ("context", 7, 12, "  return cents - fee;"),
    ("context", 8, 13, "}"),
]
rows += [{"kind": kind, "old": old, "new": new, "text": text} for kind, old, new, text in lines]
paths = ["src/payments/fees.ts", "src/payments/settlement.ts", "tests/fees.test.ts", "README.md"]
files = [{"path": path, "status": " M", "rows": rows if index == 0 else [], "loaded": index == 0, "truncated": False}
         for index, path in enumerate(paths)]
comments = [
    {"id": "rounding", "path": paths[0], "body": "Can we add a one-cent payment test? This should round up without losing precision.",
     "resolved": False, "read": False, "stale": False, "delivery": None, "position": {"side": "new", "start": 6, "end": 6}},
    {"id": "tests", "path": paths[2], "body": "Cover the maximum fee as well as the smallest payment.",
     "resolved": False, "read": True, "stale": False, "delivery": None, "position": None},
]
data = {"snapshot": {"scope": scope, "branch": "feature/payment-rounding", "head": "demo-head", "revision": "demo-v1", "files": files,
                     "limited": False, "bases": ["main", "develop"], "view": {"file": paths[0], "context": 3}}, "comments": comments}
threads = [
    {"id": "rounding-thread", "fingerprint": "demo-thread-v1", "path": paths[0], "line": 6, "side": "RIGHT",
     "resolved": False, "outdated": False, "incomplete": False, "read": False, "comments": [
         {"author": "morgan", "body": "What happens for a one-cent payment? Please add a boundary test before merging."},
         {"author": "devon", "body": "Good catch. The previous implementation rounded the fee down to zero."}]},
    {"id": "cap-thread", "fingerprint": "demo-cap-v1", "path": paths[2], "line": 24, "side": "RIGHT",
     "resolved": False, "outdated": False, "incomplete": False, "read": True, "comments": [
         {"author": "morgan", "body": "Please include a test for the maximum fee. We should keep the cap at 2,500 cents."}]},
]
github = {"pull_request": {"number": 42, "url": "https://github.com/example/payments/pull/42"}, "threads": threads}
base = (source / "main.js").read_text()
fixed = {"scope": scope, "space": "catalog-demo", "tab": "catalog-demo"}
base = base.replace("function location() {", "function location() { return " + json.dumps(fixed) + ";", 1)
base = re.sub(r"^    cx\.timer\.every\(.*\n", "", base, flags=re.MULTILINE)
base = base.replace("this.syncSidebar();", "")
base = base.replace("setShared(", "screenshotSetShared(") + "\nfunction screenshotSetShared() {}\n"

try:
    variants = [("full", "preview.png", 1440), ("github", "github-replies.png", 1280),
                ("sidebar", "sidebar.png", 440), ("scope", "scope-picker.png", 440),
                ("comment", "comment.png", 1440), ("sent", "sent.png", 1440), ("ask", "ask.png", 1440)]
    for variant, name, width in variants:
        driver = Path(tempfile.mkdtemp(prefix=f"annotator-catalog-{variant}-", dir=app_home / "mods"))
        drivers.append(driver)
        for item in source.glob("*.js"):
            if item.name != "main.js":
                script = item.read_text().replace("setShared(", "screenshotSetShared(")
                (driver / item.name).write_text(script + "\nfunction screenshotSetShared() {}\n")
        seed = "\n".join([
            "this.context = contextKey(); this.data = " + json.dumps(data) + ";",
            "this.github = " + json.dumps(github) + ";",
            "this.full = " + json.dumps(variant not in {"sidebar", "scope"}) + "; this.view = " + json.dumps("github" if variant == "github" else "diff") + ";",
            "this.open = 'src/payments/fees.ts'; this.mode = 'branch'; this.base = 'main'; this.baseExplicit = true;",
            "this.filesOpen = true; this.commentsOpen = false; this.onlyUnread = false; this.diffStyle = 'split'; this.syntax = true;",
            "this.fullWidths = [230,1050]; this.sizes = [190,420,190]; this.prepareSyntax(this.data.snapshot);",
            "this.refresh = () => {}; this.refreshGithub = () => {}; this.rememberFull = () => {}; this.rememberBase = () => {};",
            "this.currentSession = () => ({address:'demo-agent',status:'idle',app:'opencode'});",
            "this.openFull = () => {};",
            "this.consumeOpenRequest = () => {}; this.handleCommand = () => {}; this.sendQuestion = () => {}; this.sendToSession = () => {};",
            "Object.defineProperty(this, 'showResolved', { get: () => false });",
            "this.request = async () => { throw new Error('Screenshot fixture: writes and network access are disabled'); };",
        ])
        if variant == "github":
            seed += "\nthis.beginGithubReply(this.github.threads[0], cx); [...this.githubDrafts.values()][0].editor.set_value('Added the one-cent case and capped the maximum fee. The updated tests cover both boundaries.');"
        elif variant == "scope":
            seed += "\nthis.scopeMenuOpen = true;"
        elif variant == "comment":
            seed += "\nthis.draft.anchor({side:'new',start:6,end:7,origin:6}, 'demo-v1'); this.editor.set_value('Please test the smallest payment and the maximum fee cap.');"
        elif variant == "sent":
            seed += "\nthis.data.comments = this.data.comments.map(c => ({...c, delivery:'sent'})); this.notice = 'Feedback submitted to the source session. Sent comments are saved under Show sent/resolved.';"
        elif variant == "ask":
            seed += "\nthis.beginQuestion({kind:'selection',selection:{side:'new',start:6,end:7}}, cx); this.questionDraft.editor.set_value('Why do we round up here, rather than use Math.round?');"
        (driver / "mod.json").write_text(json.dumps({"place": "none", "overlay": {"width": width, "height": 800}}))
        (driver / "main.js").write_text(base.replace("this.refresh(cx);", seed, 1))
        subprocess.run(["herdr-app", "check", str(driver)], check=True)
        time.sleep(1.5)  # Let Herdr discover this fresh mod before opening it.
        subprocess.run(["herdr-app", "do", "mod.open", json.dumps({"name": driver.name})], check=True)
        def shown():
            status = json.loads(subprocess.check_output(["herdr-app", "status"]))
            return driver.name in json.dumps(status.get("overlays", []))
        try:
            time.sleep(1.5)  # Let the native overlay lay out and draw its first frame.
            if not shown():
                raise RuntimeError("Demo overlay is not shown; refusing to capture the live workspace")
            raw = output / f"{variant}-raw.png"
            subprocess.run(["herdr-app", "shot", str(raw)], check=True)
            if not shown():
                raise RuntimeError("Demo overlay closed during capture; refusing to publish the frame")
            subprocess.run(["sips", "--cropToHeightWidth", "800", str(width), str(raw), "--out", str(output / name)], check=True)
        finally:
            if shown():
                subprocess.run(["herdr-app", "do", "mod.open", json.dumps({"name": driver.name})], check=True)
finally:
    for driver in drivers:
        shutil.rmtree(driver)  # Only this invocation's newly created, inert mods.
print(f"Saved native screenshots to {output}")
