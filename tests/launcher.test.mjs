import test from "node:test";
import assert from "node:assert/strict";
import vm from "node:vm";
import { readFileSync } from "node:fs";
import * as review from "../mods/annotator/review.js";
import { runtimeConfig } from "../mods/annotator/runtime.js";

function harness(machine = null) {
  const prefix = machine ? `${machine}/` : "";
  const state = { commands: [], shared: {}, jobs: [], failure: false, repo: "/checkout",
    tabs: [{ id: `${prefix}t1`, active: true }], panes: [] };
  const owner = {};
  const cx = { notify() {}, spawn(fn) { state.jobs.push(() => fn(cx)); } };
   const context = { ...review, runtimeConfig, home: () => "/home/test",
    spaces: () => [{ id: `${prefix}s1`, active: true, machine, folder: state.repo, tabs: state.tabs }],
    panes: () => state.panes, focused: () => ({ space: `${prefix}s1`, tab: `${prefix}t1`, pane: `${prefix}p1` }),
    agents: () => [{ address: `${prefix}a1`, space: `${prefix}s1`, tab: `${prefix}t1`, pane: `${prefix}p1` }],
    layout: () => null, shared: key => state.shared[key], setShared: (key, value) => { state.shared[key] = value; },
    run: (name, args) => state.commands.push({ name, args }),
    exec: async (binary, argv) => {
      state.commands.push({ binary, argv });
      if (state.failure) throw new Error("offline");
      return { code: 0, stdout: JSON.stringify({ tab: "t2", pane: "p2" }) };
    },
  };
  const source = readFileSync(new URL("../mods/annotator/launcher.js", import.meta.url), "utf8")
    .replace(/^import .*;\n/gm, "").replace("export function openReview", "function openReview");
  vm.runInNewContext(source, context);
  return { state, owner, click: () => context.openReview(owner, cx), drain: async () => { while (state.jobs.length) await state.jobs.shift()(); } };
}

for (const machine of [null, "remote"]) test(`direct full review pins source without sidebar commands (${machine ?? "local"})`, async () => {
  const h = harness(machine); h.click(); h.click();
  await h.drain();
  assert.equal(h.state.commands.length, 2, "duplicate click creates one tab");
  const args = JSON.parse(h.state.commands[0].argv[2]);
  const prefix = machine ? `${machine}/` : "";
  assert.equal(args.mod, "annotator"); assert.equal(args.props.presentation, "full");
  assert.deepEqual(args.props.scope, { repo: "/checkout", machine });
   assert.equal(args.props.tab, `${prefix}t1`); assert.equal(args.props.source_target, `${prefix}a1`);
   assert.equal(args.props.runtime.helper, "/home/test/.local/bin/herdr-annotator");
  assert.equal(h.state.commands[1].name, "pane.focus");
  assert.equal(h.state.commands[1].args.pane, `${prefix}p2`);
  assert.equal(h.state.shared["annotator.fullTabs"][`${prefix}t2`].sourceTab, `${prefix}t1`);
});

test("missing checkout does not create a tab; failed launch allows explicit retry", async () => {
  const h = harness(); h.state.repo = ""; h.click(); await h.drain();
  assert.equal(h.state.commands.length, 0); assert.equal(h.owner.reviewError, "Select a checkout");
  h.state.repo = "/checkout"; h.state.failure = true; h.click(); await h.drain();
  assert.match(h.owner.reviewError, /offline/); assert.equal(h.owner.openingReview, false);
  h.state.failure = false; h.click(); await h.drain();
  assert.equal(h.owner.reviewError, null); assert.equal(h.state.commands.at(-1).name, "pane.focus");
});

for (const machine of [null, "remote"]) test(`open-review reuses its live tab instead of creating duplicates (${machine ?? "local"})`, async () => {
  const h = harness(machine), prefix = machine ? `${machine}/` : "";
  h.click(); await h.drain();
  h.state.tabs.push({ id: `${prefix}t2` });
  h.state.panes.push({ tab: `${prefix}t2`, pane: `${prefix}p2`, mod: "annotator" });
  h.click(); await h.drain();
  assert.equal(h.state.commands.filter(command => command.binary).length, 1);
  assert.equal(h.state.commands.at(-1).name, "pane.focus");
  assert.equal(h.state.commands.at(-1).args.pane, `${prefix}p2`);
  // A closed review, or a new checkout in the same source tab, needs a new tab.
  h.state.tabs.pop(); h.click(); await h.drain();
  assert.equal(h.state.commands.filter(command => command.binary).length, 2);
  h.state.tabs.push({ id: `${prefix}t2` }); h.state.repo = "/different";
  h.click(); await h.drain();
  assert.equal(h.state.commands.filter(command => command.binary).length, 3);
});
