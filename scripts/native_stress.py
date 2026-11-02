#!/usr/bin/env python3
"""Render a populated large review under Herdr's real JavaScript budget. Never sends."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--files", type=int, default=5000)
parser.add_argument("--full", action="store_true", help="Check the standalone full-review presentation")
parser.add_argument("--branch", action="store_true", help="Render Branch controls including the base selector")
parser.add_argument("--rows", type=int, default=0, help="Source rows in the selected diff")
parser.add_argument("--parent", help="Parent directory for temporary native fixture")
args = parser.parse_args()
source = Path(__file__).resolve().parents[1] / "mods/annotator"
data = {"snapshot": {"scope": {"repo": "/fixture", "machine": None}, "branch": "fixture",
    "revision": "fixture", "files": [{"path": f"src/dir/file-{n:05}.rs", "status": "M",
    "rows": [], "loaded": False, "truncated": False} for n in range(args.files)]}, "comments": []}
if data["snapshot"]["files"]:
    data["snapshot"]["files"][0]["loaded"] = True
    data["snapshot"]["files"][0]["rows"] = [{"kind": "context", "old": n + 1,
        "new": n + 1, "text": f"source line {n + 1}"} for n in range(args.rows)]
with tempfile.TemporaryDirectory(prefix="annotator-stress-", dir=args.parent) as temp:
    target = Path(temp)
    shutil.copy2(source / "review.js", target / "review.js")
    shutil.copy2(source / "full.js", target / "full.js")
    shutil.copy2(source / "syntax.js", target / "syntax.js")
    shutil.copy2(source / "draft.js", target / "draft.js")
    script = (source / "main.js").read_text()
    script = script.replace("this.refresh(cx);", "this.full = " + json.dumps(args.full) + "; this.mode = " + json.dumps("branch" if args.branch else "uncommitted") + "; this.base = 'origin/develop'; this.context = contextKey(); this.data = " + json.dumps(data) + "; this.open = this.data.snapshot.files[0]?.path; this.prepareSyntax(this.data.snapshot);", 1)
    (target / "main.js").write_text(script)
    result = subprocess.run(["herdr-app", "check", str(target)])
    raise SystemExit(result.returncode)
