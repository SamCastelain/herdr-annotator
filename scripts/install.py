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
import sys
from tabs_integration import integrated_tabs
from install_support import install_paths, replace_helper

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--layout", action="append", default=[], help="Personal layout whose Changes panel should use this mod; repeat for multiple layouts")
parser.add_argument("--expected-commit", help=argparse.SUPPRESS)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
home = Path.home()
try:
    app_home, binary, receipt = install_paths(home, os.environ)
except ValueError as error:
    parser.error(str(error))
mod = app_home / "mods/annotator"
updater = receipt.parent / "update.py"
installed = json.loads(receipt.read_text(encoding="utf-8")) if receipt.exists() else {}
files = ["mod.json", "review.js", "main.js", "full.js", "syntax.js", "draft.js", "github-refresh.js"]
files.append("updates.js")
files.append("runtime.js")
runtime = {"helper": str(binary), "python": sys.executable,
           "script": str(updater), "updatesEnabled": os.name != "nt"}
runtime_source = (root / "mods/annotator/runtime.js").read_text(encoding="utf-8")
if runtime_source.count("export const installation = {};") != 1:
    parser.error("Unsupported runtime configuration template; no files were installed")
runtime_source = runtime_source.replace(
    "export const installation = {};", "export const installation = " + json.dumps(runtime, ensure_ascii=True) + ";", 1)
def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()
if updater.exists() and (not installed.get("updater_digest") or digest(updater) != installed["updater_digest"]):
    parser.error(f"Installed updater changed locally or has no receipt: {updater}")
version = json.loads((root / "mods/annotator/mod.json").read_text(encoding="utf-8"))["version"]
tools = {name: str(Path(path).resolve()) for name in ("git", "gh", "opencode", "pi", "node", "hc")
         if (path := shutil.which(name))}
commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, check=True, capture_output=True, text=True, encoding="utf-8").stdout.strip()
def check_release_source():
    if args.expected_commit:
        head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=root, check=True, capture_output=True, text=True, encoding="utf-8").stdout.strip()
        dirty = subprocess.run(["git", "status", "--porcelain=v1", "--untracked-files=all"], cwd=root, check=True, capture_output=True, text=True, encoding="utf-8").stdout.strip()
        if head != args.expected_commit or dirty:
            parser.error("Release checkout changed during the update; no installed files were replaced")
check_release_source()
layout = None
layouts = []
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
    if installed.get("binary_digest") and digest(binary) != installed["binary_digest"]:
        parser.error(f"Installed helper changed locally; preserve/reconcile it first: {binary}")
for layout_name in dict.fromkeys(args.layout):
    if layout_name in {"main", "terminals", "win98"} or Path(layout_name).name != layout_name:
        parser.error("Choose a personal layout name, not a shipped layout or a path")
    layout = app_home / "layouts" / f"{layout_name}.json"
    if not layout.is_file():
        parser.error(f"Personal layout does not exist: {layout}")
    tree = json.loads(layout.read_text(encoding="utf-8"))
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
    layouts.append((layout, tree))
if layouts or installed.get("tabs_files"):
    try:
        tabs_source = integrated_tabs((app_home / "mods/tabs/main.js").read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        parser.error(str(error))
    tabs_files = {"main.js": tabs_source, "mod.json": '{"place":"none","title":"Annotator tabs"}\n',
                  **{name: (root / "mods/annotator" / name).read_text(encoding="utf-8") for name in ["launcher.js", "review.js"]},
                  "runtime.js": runtime_source}
    if tabs_mod.exists() or tabs_mod.is_symlink():
        if tabs_mod.is_symlink() or installed.get("repo") != str(root) or not installed.get("tabs_files"):
            parser.error(f"Existing private tabs mod has no matching receipt: {tabs_mod}")
        for name in tabs_files:
            if name not in installed["tabs_files"] and not (tabs_mod / name).exists():
                continue
            if not (tabs_mod / name).is_file() or digest(tabs_mod / name) != installed["tabs_files"].get(name):
                parser.error(f"Private tabs mod changed locally; preserve/reconcile it first: {tabs_mod / name}")
    with tempfile.TemporaryDirectory(prefix="annotator-tabs-") as stage:
        for name, content in tabs_files.items():
            (Path(stage) / name).write_text(content, encoding="utf-8")
        subprocess.run(["herdr-app", "check", stage], check=True)
subprocess.run(["cargo", "build", "--release", "--locked"], cwd=root, check=True)
with tempfile.TemporaryDirectory(prefix="annotator-check-") as stage:
    for name in files:
        if name == "runtime.js":
            (Path(stage) / name).write_text(runtime_source, encoding="utf-8")
        else:
            shutil.copy2(root / "mods/annotator" / name, Path(stage) / name)
    subprocess.run(["herdr-app", "check", stage], check=True)
check_release_source()
stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
backups = receipt.parent / "backups" / stamp
if binary.exists() or layout or mod.exists():
    backups.mkdir(parents=True)
    if binary.exists():
        shutil.copy2(binary, backups / binary.name)
    if updater.exists():
        shutil.copy2(updater, backups / "update.py")
    if receipt.exists():
        shutil.copy2(receipt, backups / "install.json")
    for layout_path, _ in layouts:
        shutil.copy2(layout_path, backups / layout_path.name)
    if mod.is_symlink():
        (backups / "annotator-link-target.txt").write_text(str(mod.readlink()) + "\n", encoding="utf-8")
    elif mod.exists():
        shutil.copytree(mod, backups / "annotator")
    if tabs_files and tabs_mod.exists():
        shutil.copytree(tabs_mod, backups / "annotator-tabs")
binary.parent.mkdir(parents=True, exist_ok=True)
replace_helper(root / "target/release" / binary.name, binary, stamp)
mod.parent.mkdir(parents=True, exist_ok=True)
if mod.is_symlink() or not mod.exists():
    stage = mod.with_name(f".annotator-{stamp}")
    stage.mkdir()
    for name in files:
        if name == "runtime.js":
            (stage / name).write_text(runtime_source, encoding="utf-8")
        else:
            shutil.copy2(root / "mods/annotator" / name, stage / name)
    if mod.is_symlink():
        os.replace(mod, backups / "annotator-link")
    os.replace(stage, mod)
else:
    for name in files:
        temp = mod / f".{name}-{stamp}"
        if name == "runtime.js":
            temp.write_text(runtime_source, encoding="utf-8")
        else:
            shutil.copy2(root / "mods/annotator" / name, temp)
        os.replace(temp, mod / name)
receipt.parent.mkdir(parents=True, exist_ok=True)
if tabs_files:
    tabs_mod.mkdir(parents=True, exist_ok=True)
    for name, content in tabs_files.items():
        temp = tabs_mod / f".{name}-{stamp}"
        temp.write_text(content, encoding="utf-8")
        os.replace(temp, tabs_mod / name)
    installed["tabs_files"] = {name: digest(tabs_mod / name) for name in tabs_files}
updater_temp = updater.with_name(f".update-{stamp}.py")
shutil.copy2(root / "scripts/update.py", updater_temp)
os.replace(updater_temp, updater)
receipt_temp = receipt.with_name(f".install-{stamp}.json")
receipt_temp.write_text(json.dumps({"repo": str(root), "binary": str(binary), "mod": str(mod), "app_home": str(app_home),
                              "version": version, "commit": commit, "binary_digest": digest(binary), "updater_digest": digest(updater),
                              "layouts": sorted(set(installed.get("layouts", [])) | set(args.layout)), "tools": tools, "python": sys.executable, "runtime": runtime,
                              "files": {name: digest(mod / name) for name in files},
                              **({"tabs_files": installed["tabs_files"]} if installed.get("tabs_files") else {})}, indent=2) + "\n", encoding="utf-8")
os.replace(receipt_temp, receipt)
for layout_path, tree in layouts:
    temp = layout_path.with_name(f".{layout_path.name}-{stamp}")
    temp.write_text(json.dumps(tree, indent=2) + "\n", encoding="utf-8")
    os.replace(temp, layout_path)
print(f"Installed {binary}\nInstalled mod copy at {mod}")
if layouts:
    print(f"Updated {', '.join(str(path) for path, _ in layouts)}; backup in {backups}")
else:
    print("No layout changed. Existing Annotator panels reload; use ↗ in Changes to open a full review tab.")
