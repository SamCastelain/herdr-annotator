#!/usr/bin/env python3
"""Open an isolated fixture through the real Full review action. Never sends."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
parent = Path(tempfile.gettempdir()) / "opencode"
parent.mkdir(exist_ok=True)
fixture = json.loads(subprocess.check_output(["python3", str(root / "scripts/native_fixture.py"), "--parent", str(parent)], text=True))
status = json.loads(subprocess.check_output(["herdr-app", "status"], text=True))
app_home = Path(os.environ.get("HERDR_APP_HOME", Path.home() / ".config/herdr-app"))
driver = Path(tempfile.mkdtemp(prefix="annotator-smoke-", dir=app_home / "mods"))
record = Path(fixture["fixture"]) / "created-review.json"
scope = {"scope": fixture["scope"], "space": status["space"], "tab": status["tab"]}
request = {"op": "snapshot", "scope": fixture["scope"], "view": {"file": "payment.rs", "context": 3}}
data = json.loads(subprocess.check_output([fixture["helper"], "request", json.dumps(request)], text=True))["data"]
script = (root / "mods/annotator/main.js").read_text()
script = script.replace("function location() {", "function location() { return " + json.dumps(scope) + ";", 1)
once = json.dumps("annotator.smoke." + driver.name)
auto_open = "if (!shared(" + once + ")) { setShared(" + once + ", true); this.openFull(cx); }"
script = script.replace("this.refresh(cx);", "this.context = contextKey(); this.data = " + json.dumps(data) + "; this.open = 'payment.rs'; " + auto_open, 1)
script = script.replace('source_target: target?.address ?? null', 'source_target: "annotator-smoke-no-agent"')
script = script.replace('presentation: "full", scope:', 'presentation: "full", helper: ' + json.dumps(fixture["helper"]) + ', scope:')
record_call = 'await exec("python3", ["-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).write_text(sys.argv[2])", ' + json.dumps(str(record)) + ', JSON.stringify({tab,pane})]);'
script = script.replace('run("pane.focus", { pane });', 'run("pane.focus", { pane }); ' + record_call)
(driver / "main.js").write_text(script.replace(auto_open, "", 1))
for name in ["review.js", "full.js", "syntax.js", "draft.js"]:
    shutil.copy2(root / "mods/annotator" / name, driver / name)
(driver / "mod.json").write_text(json.dumps({"place": "none", "overlay": {"width": 360, "height": 300}}))
subprocess.run(["herdr-app", "check", str(driver)], check=True)
(driver / "main.js").write_text(script)
subprocess.run(["herdr-app", "do", "mod.open", json.dumps({"name": driver.name})], check=True)
print(json.dumps({"driver": str(driver), "record": str(record), "source": status, "fixture": fixture}, indent=2))
