import test from "node:test";
import assert from "node:assert/strict";
import { runtimeConfig } from "../mods/annotator/runtime.js";

test("runtime configuration preserves Unix defaults and explicit helper overrides", () => {
  assert.equal(runtimeConfig("/home/test").helper, "/home/test/.local/bin/herdr-annotator");
  const runtime = { helper: "C:\\tools\\herdr-annotator.exe", python: "C:\\Python\\python.exe",
    script: "C:\\state\\update.py", updatesEnabled: false };
  assert.deepEqual(runtimeConfig("unused", { runtime }), runtime);
  assert.equal(runtimeConfig("unused", { runtime, helper: "explicit" }).helper, "explicit");
  assert.deepEqual(runtimeConfig("unused", { runtime: runtimeConfig("unused", { runtime }) }), runtime);
});

test("runtime paths preserve Unicode, spaces, parentheses, quotes and shell metacharacters", () => {
  const runtime = { helper: "C:\\tools (中文) & 'quotes' %PATH%\\herdr-annotator.exe",
    python: "C:\\Python (中文)\\python.exe", script: "C:\\state & tools\\update.py", updatesEnabled: false };
  assert.deepEqual(runtimeConfig("unused", { runtime }), runtime);
});
