#!/usr/bin/env python3
"""Build and install the mod locally. Existing unrelated installs are never replaced."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
from tabs_integration import integrated_tabs

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--layout", help="Optional personal layout whose Changes panel should use this mod")
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
home = Path.home()
app_home = Path(os.environ.get("HERDR_APP_HOME", home / ".config/herdr-app"))
mod = app_home / "mods/annotator"
binary = home / ".local/bin/herdr-annotator"
receipt = home / ".local/share/herdr-annotator/install.json"
installed = json.loads(receipt.read_text()) if receipt.exists() else {}
files = ["mod.json", "review.js", "main.js", "full.js", "syntax.js", "draft.js", "github-refresh.js"]
def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()
layout = None
tabs_mod = app_home / "mods/annotator-tabs"
tabs_files = None
if mod.is_symlink():
    if mod.resolve() != root / "mods/annotator":
        parser.error(f"Another annotator mod exists at {mod}; choose a different name first")
elif mod.exists():
    if installed.get("repo") != str(root) or not installed.get("files"):
        parser.error(f"Existing mod has no matching install receipt: {mod}")
    for name in files:
        if name not in installed["files"] and not (mod / name).exists():
            continue  # A new source file in an upgrade; existing files still require a receipt.
        if not (mod / name).is_file() or digest(mod / name) != installed["files"].get(name):
            parser.error(f"Installed mod changed locally; preserve/reconcile it before updating: {mod / name}")
if binary.exists():
    if installed.get("repo") != str(root):
        parser.error(f"Existing binary has no matching install receipt: {binary}")
if args.layout:
    if args.layout in {"main", "terminals", "win98"} or Path(args.layout).name != args.layout:
        parser.error("Choose a personal layout name, not a shipped layout or a path")
    layout = app_home / "layouts" / f"{args.layout}.json"
    if not layout.is_file():
        parser.error(f"Personal layout does not exist: {layout}")
    tree = json.loads(layout.read_text())
    def replace(node):
        count = 0
        if node.get("content", {}).get("mod") in {"changes", "annotator"}:
            node["content"]["mod"] = "annotator"
            count += 1
        for child in node.get("children", []):
            count += replace(child)
        return count
    if not replace(tree):
        parser.error("No stock Changes panel found; no layout change needed or this layout uses another mod")
    def replace_tabs(node):
        count = 0
        if node.get("content", {}).get("mod") in {"tabs", "annotator-tabs"}:
            node["content"]["mod"] = "annotator-tabs"
            count += 1
        for child in node.get("children", []):
            count += replace_tabs(child)
        return count
    if not replace_tabs(tree):
        parser.error("No supported tabs mod found in this layout")
    try:
        tabs_source = integrated_tabs((app_home / "mods/tabs/main.js").read_text())
    except (OSError, ValueError) as error:
        parser.error(str(error))
    tabs_files = {"main.js": tabs_source, "mod.json": '{"place":"none","title":"Annotator tabs"}\n',
                  **{name: (root / "mods/annotator" / name).read_text() for name in ["launcher.js", "review.js"]}}
    if tabs_mod.exists() or tabs_mod.is_symlink():
        if tabs_mod.is_symlink() or installed.get("repo") != str(root) or not installed.get("tabs_files"):
            parser.error(f"Existing private tabs mod has no matching receipt: {tabs_mod}")
        for name in tabs_files:
            if not (tabs_mod / name).is_file() or digest(tabs_mod / name) != installed["tabs_files"].get(name):
                parser.error(f"Private tabs mod changed locally; preserve/reconcile it first: {tabs_mod / name}")
    with tempfile.TemporaryDirectory(prefix="annotator-tabs-") as stage:
        for name, content in tabs_files.items():
            (Path(stage) / name).write_text(content)
        subprocess.run(["herdr-app", "check", stage], check=True)
subprocess.run(["cargo", "build", "--release", "--locked"], cwd=root, check=True)
subprocess.run(["herdr-app", "check", str(root / "mods/annotator")], check=True)
stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
backups = home / ".local/share/herdr-annotator/backups" / stamp
if binary.exists() or layout or mod.exists():
    backups.mkdir(parents=True)
    if binary.exists():
        shutil.copy2(binary, backups / "herdr-annotator")
    if layout:
        shutil.copy2(layout, backups / layout.name)
    if mod.is_symlink():
        (backups / "annotator-link-target.txt").write_text(str(mod.readlink()) + "\n")
    elif mod.exists():
        shutil.copytree(mod, backups / "annotator")
    if tabs_files and tabs_mod.exists():
        shutil.copytree(tabs_mod, backups / "annotator-tabs")
binary.parent.mkdir(parents=True, exist_ok=True)
temp = binary.with_name(f".herdr-annotator-{stamp}")
shutil.copy2(root / "target/release/herdr-annotator", temp)
os.replace(temp, binary)
mod.parent.mkdir(parents=True, exist_ok=True)
if mod.is_symlink() or not mod.exists():
    stage = mod.with_name(f".annotator-{stamp}")
    stage.mkdir()
    for name in files:
        shutil.copy2(root / "mods/annotator" / name, stage / name)
    if mod.is_symlink():
        os.replace(mod, backups / "annotator-link")
    os.replace(stage, mod)
else:
    for name in files:
        temp = mod / f".{name}-{stamp}"
        shutil.copy2(root / "mods/annotator" / name, temp)
        os.replace(temp, mod / name)
receipt.parent.mkdir(parents=True, exist_ok=True)
if tabs_files:
    tabs_mod.mkdir(parents=True, exist_ok=True)
    for name, content in tabs_files.items():
        temp = tabs_mod / f".{name}-{stamp}"
        temp.write_text(content)
        os.replace(temp, tabs_mod / name)
    installed["tabs_files"] = {name: digest(tabs_mod / name) for name in tabs_files}
receipt.write_text(json.dumps({"repo": str(root), "binary": str(binary), "mod": str(mod),
                              "files": {name: digest(mod / name) for name in files},
                              **({"tabs_files": installed["tabs_files"]} if installed.get("tabs_files") else {})}, indent=2) + "\n")
if layout:
    temp = layout.with_name(f".{layout.name}-{stamp}")
    temp.write_text(json.dumps(tree, indent=2) + "\n")
    os.replace(temp, layout)
print(f"Installed {binary}\nInstalled mod copy at {mod}")
if layout:
    print(f"Updated {layout}; backup in {backups}")
else:
    print("No layout changed. Existing Annotator panels reload; use ↗ in Changes to open a full review tab.")
