import test from "node:test";
import assert from "node:assert/strict";
import { sessionLabel } from "../mods/annotator/review.js";

test("recipient labels use the exact source space and tab, not another machine or review tab", () => {
  const target = { address: "wsl/t1r", space: "wsl/s1", tab: "wsl/tab1" };
  const spaces = [
    { id: "s1", tabs: [{ id: "tab1", name: "Windows tab" }] },
    { id: "wsl/s1", tabs: [{ id: "wsl/tab1", name: "Source workflow" }, { id: "wsl/review", name: "Review" }] },
  ];
  assert.equal(sessionLabel(target, spaces), "Source workflow");
  assert.equal(sessionLabel(target, spaces.slice(0, 1)), "wsl/t1r");
  assert.equal(sessionLabel(target, []), "wsl/t1r");
  assert.equal(sessionLabel(null, spaces), "");
});

test("missing or empty tab names fall back to the address", () => {
  const target = { address: "t1r", space: "s1", tab: "tab1" };
  for (const name of [undefined, "", "   "]) {
    assert.equal(sessionLabel(target, [{ id: "s1", tabs: [{ id: "tab1", name }] }]), "t1r");
  }
  assert.equal(sessionLabel(target, [{ id: "s1", tabs: [{ id: "tab1", title: "Source title" }] }]), "Source title");
});
