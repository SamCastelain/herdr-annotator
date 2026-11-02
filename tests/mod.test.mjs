import test from "node:test";
import assert from "node:assert/strict";
import vm from "node:vm";
import { readFileSync } from "node:fs";
import * as review from "../mods/annotator/review.js";
import * as syntax from "../mods/annotator/syntax.js";
import { DraftLifecycle } from "../mods/annotator/draft.js";

function harness() {
  const nodes = [], requests = [], jobs = [];
  const state = { scope: { repo: "/checkout", machine: null }, revision: "r1", reply: null,
    tab: "tab1", focus: { terminal: "t7", pane: "p7" }, commands: [], props: {}, shared: {}, layout: null, machines: [],
    agents: [{ address: "t7", pane: "p7", space: "space1", tab: "tab1", status: "idle" }] };
  function node() {
    const data = { children: [], events: {} };
    const proxy = new Proxy(data, { get(_target, key) {
      if (key === "then") return undefined;
      return (...args) => {
        if (key === "id") data.id = args[0];
        else if (key === "child") data.children.push(args[0]);
        else if (key === "children") data.children.push(...args[0]);
        else if (key === "when" && args[0]) args[1](proxy);
        else if (key.startsWith("on_")) data.events[key] = args.at(-1);
        return proxy;
      };
    }});
    nodes.push(data); return proxy;
  }
  function input(options) {
    let value = options.value ?? "";
    const events = {};
    return { value: () => value, set_value(v) { value = v; }, on(name, fn) { events[name] = fn; },
      focus() { events.focus?.({}, cx); }, emit(name, event) { events[name]?.(event, cx); }, release() {} };
  }
  const snapshot = () => ({
    snapshot: { scope: { ...state.scope }, branch: "main", revision: state.revision,
      files: [{ path: "a.rs", status: " M", rows: [{ kind: "add", old: null, new: 2, text: "changed" }], truncated: false }], limited: false },
    comments: [{ id: "note1", path: "a.rs", body: "Inline note", stale: false, resolved: false, delivery: null, position: { side: "new", start: 2, end: 2 } }],
  });
  const cx = { notify() {}, stop_propagation() {}, focus_handle: () => ({}), timer: { every() {} }, spawn(fn) { jobs.push(Promise.resolve().then(() => fn(cx))); } };
  const context = {
    ...review, ...syntax, DraftLifecycle, View: class {}, div: node, h_flex: node, v_flex: node, v_resizable: node, h_resizable: node,
    resizable_panel: () => { state.panelCalls = (state.panelCalls ?? 0) + 1; return node(); },
    shared: key => state.shared[key] ?? null, setShared: (key, value) => { state.saved = { key, value }; state.shared[key] = value; },
    layout: () => state.layout,
    machines: () => state.machines,
    appHome: () => "/app-home", watchFolder: async () => { throw new Error("Native file events unavailable in unit harness"); },
    v_virtual_list: (id, count, size, key, render) => {
      const result = node().id(id);
      state.virtual = { id, count, size, key, render };
      return result;
    },
    Scrollbar: { vertical: node },
    list: (id, count, key, render) => { const result = node().id(id); state.diffList = { id, count, key, render }; return result; },
    run: (name, args) => { state.commands.push({ name, args }); },
    TextareaState: { new: input }, Textarea: { new: node },
    props: () => ({ scope: state.scope, space: "space1", tab: state.tab, ...state.props }),
    theme: () => ({ dim: "#666666", ansi1: "#ff0000", ansi2: "#00ff00" }), settings: () => ({}),
    agents: () => state.agents, focused: () => state.focus,
    home: () => "/home/me", exec: async (_helper, args) => {
      if (_helper === "herdr-app") {
        state.commands.push({ name: args[1], args: JSON.parse(args[2]) });
        return { code: 0, stdout: JSON.stringify({ tab: "review-tab", pane: "review-pane" }) };
      }
      const request = JSON.parse(args[1]); requests.push(request);
      if (state.reply) return state.reply(request);
      return { code: 0, stdout: JSON.stringify({ ok: true, data: snapshot() }) };
    },
  };
  const source = readFileSync(new URL("../mods/annotator/main.js", import.meta.url), "utf8")
    .replace(/^import .*;\n/gm, "").replace("export default class Annotator", "globalThis.Annotator = class Annotator");
  const full = readFileSync(new URL("../mods/annotator/full.js", import.meta.url), "utf8")
    .replace(/^import .*;\n/gm, "").replace("export function renderFull", "function renderFull");
  vm.runInNewContext(full, context);
  vm.runInNewContext(source, context);
  const mod = new context.Annotator();
  const drain = async () => { while (jobs.length) await Promise.all(jobs.splice(0)); };
  return { mod, nodes, requests, state, cx, drain, snapshot };
}

test("one click sends saved comments; rendering alone never sends", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain(); h.mod.render(h.cx);
  assert.ok(h.nodes.some(n => n.id === "diff-0"));
  assert.ok(h.nodes.some(n => n.id === "note-note1"));
  assert.ok(h.nodes.some(n => n.id === "save-note"));
   assert.equal(h.requests.length, 2); // Snapshot and read-only GitHub fetch; rendering never sends.
  assert.equal(h.mod.currentSession().address, "t7");
  h.nodes.filter(n => n.id === "send-review").at(-1).events.on_click(null, h.cx);
  assert.equal(h.mod.busy, true);
  h.mod.sendToSession(h.cx); // A second click during delivery cannot enqueue another send.
  await h.drain();
  assert.equal(h.requests.at(-1).op, "send"); assert.equal(h.requests.at(-1).target, "t7");
  assert.equal(h.requests.filter(r => r.op === "send").length, 1);
  h.mod.render(h.cx);
  assert.ok(!h.nodes.some(n => n.id === "cancel-send"));
});

test("resolve emits the documented request without an invalid revision field", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.mod.act("resolve", { id: "note1", resolved: true }, h.cx); await h.drain();
  assert.equal(h.requests.at(-1).op, "resolve");
  assert.equal("revision" in h.requests.at(-1), false);
});

test("refresh preserves drafts; changed previews clear bare line selections", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.mod.editor.set_value("unsaved draft"); h.mod.refresh(h.cx); await h.drain();
  assert.equal(h.requests.length, 2); assert.equal(h.mod.editor.value(), "unsaved draft");
  h.mod.editor.set_value(""); h.mod.selection = { side: "new", start: 2, end: 2, origin: 2 };
  h.state.revision = "r2"; h.mod.refresh(h.cx); await h.drain();
  assert.equal(h.mod.selection, null);
});

test("save finishing during a space switch does not resurrect a duplicate draft", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.mod.editor.set_value("to save");
  h.state.reply = async () => {
    h.state.scope = { repo: "/other", machine: null };
    return { code: 0, stdout: JSON.stringify({ ok: true, data: h.snapshot() }) };
  };
  h.mod.act("add", { path: "a.rs", selection: null, body: "to save" }, h.cx); await h.drain();
  assert.equal(h.mod.editor.value(), ""); assert.equal(h.mod.drafts.size, 0);
});

test("comments on a file that leaves the diff render once, as outdated notes", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.mod.data.snapshot.files = [];
  h.mod.data.comments[0].stale = true; h.mod.data.comments[0].position = null;
  h.mod.render(h.cx);
  assert.equal(h.nodes.filter(n => n.id === "note-note1").length, 1);
});

test("one-click send resolves the current chat rather than a stale recipient", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.state.agents.push({ address: "t8", pane: "p8", space: "space1", tab: "tab1", status: "idle" });
  h.mod.render(h.cx); // Initially displays t7.
  h.state.focus = { terminal: "t8", pane: "p8" };
  h.nodes.filter(n => n.id === "send-review").at(-1).events.on_click(null, h.cx); await h.drain();
  assert.equal(h.requests.filter(r => r.op === "send").length, 1);
  assert.equal(h.requests.at(-1).target, "t8");
});

test("one-click send still refuses unsaved drafts and empty batches", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.mod.editor.set_value("unsaved"); h.mod.sendToSession(h.cx); await h.drain();
  h.mod.editor.set_value(""); h.mod.data.comments = []; h.mod.sendToSession(h.cx); await h.drain();
  assert.equal(h.requests.filter(r => r.op === "send").length, 0);
});

test("composer focus retains the chat; a working session never receives feedback", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain(); h.mod.render(h.cx);
  h.state.focus = { terminal: null, pane: null };
  assert.equal(h.mod.currentSession().address, "t7");
  h.state.agents[0].status = "working";
  h.mod.sendToSession(h.cx); await h.drain();
  assert.equal(h.requests.filter(r => r.op === "send").length, 0);
  assert.match(h.mod.notice, /working/);
});

test("Comments tab exposes every saved note without sending or mutating", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain(); h.mod.render(h.cx);
  const stale = { ...h.mod.data.comments[0], id: "outdated", path: "gone.rs", stale: true, position: null };
  h.mod.data.comments.push(stale);
  h.nodes.find(n => n.id === "view-comments").events.on_click(null, h.cx);
  const start = h.nodes.length; h.mod.render(h.cx);
  const current = h.nodes.slice(start);
  assert.ok(current.some(n => n.id === "review-comments"));
  assert.equal(current.filter(n => n.id === "note-note1").length, 1);
  assert.equal(current.filter(n => n.id === "note-outdated").length, 1);
  assert.equal(h.requests.length, 2);
});

test("file-list collapse is local UI state and draft reselection can choose a file comment", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain(); h.mod.render(h.cx);
  h.nodes.find(n => n.id === "toggle-files").events.on_click(null, h.cx);
  const start = h.nodes.length; h.mod.render(h.cx);
  assert.ok(!h.nodes.slice(start).some(n => n.id === "review-files"));
  h.mod.editor.set_value("kept draft"); h.mod.draft.observe("changed", "r1");
  h.mod.render(h.cx);
  h.nodes.filter(n => n.id === "file-note").at(-1).events.on_click(null, h.cx);
  assert.equal(h.mod.needsSelection, false); assert.equal(h.mod.editor.value(), "kept draft");
  assert.equal(h.requests.length, 2);
});

test("stale line draft cannot become a file note after a checkout round trip", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.mod.draft.anchor({ side: "new", start: 2, end: 2 }, "r1");
  h.mod.editor.set_value("line intent"); h.mod.snapshotFailed("Connection timed out", h.state.scope);
  h.state.revision = "r2"; h.mod.refresh(h.cx, true); await h.drain();
  assert.equal(h.mod.needsSelection, true);
  h.state.scope = { repo: "/other", machine: null }; h.mod.refresh(h.cx); await h.drain();
  h.state.scope = { repo: "/checkout", machine: null }; h.mod.refresh(h.cx); await h.drain();
  assert.equal(h.mod.needsSelection, true);
  h.mod.saveComment(h.cx); await h.drain();
  assert.equal(h.requests.filter(r => r.op === "add").length, 0);
  h.mod.fileNote(h.cx); h.mod.saveComment(h.cx); await h.drain();
  assert.equal(h.requests.at(-1).selection, null);
});

test("late files and context changes request fresh lazy previews", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.mod.selectFile("file-249.rs", h.cx); await h.drain();
  assert.equal(h.requests.at(-1).view.file, "file-249.rs");
  h.mod.expandContext(23, h.cx); await h.drain();
  assert.equal(h.requests.at(-1).view.context, 23);
  h.mod.editor.set_value("draft");
  h.mod.expandContext(1000000, h.cx); await h.drain();
  assert.equal(h.mod.contextLines, 23);
});

test("Branch comparison and base are independent of Code and Comments tabs", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.mod.changeComparison("branch", "origin/main", h.cx); await h.drain();
  assert.equal(h.requests.at(-1).scope.comparison.mode, "branch");
  assert.equal(h.requests.at(-1).scope.comparison.base, "origin/main");
  h.mod.render(h.cx);
  h.nodes.filter(n => n.id === "view-comments").at(-1).events.on_click(null, h.cx);
  assert.equal(h.mod.mode, "branch");
});

test("section drag sizes persist and context controls render", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain(); h.mod.render(h.cx);
  h.nodes.find(n => n.events.on_resize).events.on_resize([210, 400, 190], h.cx);
  assert.equal(h.state.saved.key, "annotator.sections");
  assert.deepEqual(h.state.saved.value, [210, 400, 190]);
  assert.ok(h.nodes.some(n => n.id === "context-more"));
  assert.ok(h.nodes.some(n => n.id === "context-all"));
});

test("Fix thread excludes local notes; batches include queued fingerprints", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  const thread = { id: "t1", fingerprint: "fp1" };
  h.mod.sendToSession(h.cx, thread); await h.drain();
  assert.equal(h.requests.at(-1).include_local, false);
  assert.deepEqual(h.requests.at(-1).threads, [thread]);
  h.mod.threadQueue.set(thread.id, thread);
  h.mod.sendToSession(h.cx); await h.drain();
  assert.equal(h.requests.at(-1).include_local, true);
  assert.deepEqual(h.requests.at(-1).threads, [thread]);
});

test("GitHub refresh drops changed queued thread versions without sending", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.mod.threadQueue.set("t1", { id: "t1", fingerprint: "old" });
  h.state.reply = async () => ({ code: 0, stdout: JSON.stringify({ ok: true, data: { github: { threads: [{ id: "t1", fingerprint: "new" }] } } }) });
  h.mod.refreshGithub(h.cx, true); await h.drain();
  assert.equal(h.requests.at(-1).force, true);
  assert.equal(h.mod.threadQueue.size, 0);
  assert.equal(h.requests.filter(r => r.op === "send").length, 0);
});

test("large file lists build only visible rows and select by stable path", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.mod.data.snapshot.files = Array.from({ length: 5000 }, (_, i) => ({
    path: `src/file-${i}.rs`, status: "M", rows: [], loaded: false,
  }));
  let built = 0;
  const fileRow = h.mod.fileRow.bind(h.mod);
  h.mod.fileRow = (...args) => { built++; return fileRow(...args); };
  h.mod.render(h.cx);
  assert.equal(built, 0, "offscreen file rows must not be built in render()");
  assert.equal(h.state.virtual.count, 5000);
  assert.equal(h.state.virtual.key(4999), "src/file-4999.rs");
  const start = h.nodes.length;
  h.state.virtual.render({ start: 4990, end: 5000 });
  assert.equal(built, 10);
  assert.ok(h.nodes.slice(start).every(n => !n.events.on_click), "deferred native rows cannot register handlers");
  h.nodes.find(n => n.id === "review-files-list").events.on_item_click("src/file-4999.rs", h.cx);
  await h.drain();
  assert.equal(h.requests.at(-1).view.file, "src/file-4999.rs");
});

test("Open full review pins checkout, comparison, file and exact source chat", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.mod.data.snapshot.scope.comparison = { mode: "branch", base: "origin/main" };
  h.mod.openFull(h.cx); await h.drain();
  const command = h.state.commands[0];
  assert.equal(command.name, "tab.new");
  assert.equal(command.args.mod, "annotator");
  assert.equal(command.args.props.presentation, "full");
  assert.equal(command.args.props.source_target, "t7");
  assert.equal(command.args.props.tab, "tab1");
  assert.equal(command.args.props.file, "a.rs");
  assert.equal(command.args.props.scope.comparison.base, "origin/main");
  h.mod.editor.set_value("draft"); h.mod.openFull(h.cx);
  assert.equal(h.state.commands.filter(c => c.name === "tab.new").length, 1);
  assert.equal(h.state.commands.at(-1).name, "pane.focus");
  assert.equal(h.state.shared["annotator.fullTabs"]["review-tab"].sourceTab, "tab1");
});

test("full review keeps its original chat and never redirects when it closes", async () => {
  const h = harness();
  h.state.props = { presentation: "full", source_target: "t7", tab: "tab1" };
  h.mod.init({}, h.cx); await h.drain();
  h.state.agents.push({ address: "t8", pane: "p8", space: "space1", tab: "tab1", status: "idle" });
  h.state.focus = { terminal: "t8", pane: "p8" };
  assert.equal(h.mod.currentSession().address, "t7");
  h.mod.render(h.cx);
  assert.ok(h.nodes.some(n => n.id === "annotator-full"));
  assert.ok(h.nodes.some(n => n.id === "full-split"));
  h.state.agents = h.state.agents.filter(a => a.address !== "t7");
  assert.equal(h.mod.currentSession(), null);
  h.mod.sendToSession(h.cx); await h.drain();
  assert.equal(h.requests.filter(r => r.op === "send").length, 0);
});

test("full diff rows are deferred; split selection keeps old and new sides", async () => {
  const h = harness(); h.state.props = { presentation: "full" };
  h.mod.init({}, h.cx); await h.drain();
  h.mod.data.comments = [];
  h.mod.data.snapshot.files[0].rows = [
    { kind: "delete", old: 5, new: null, text: "before" },
    { kind: "add", old: null, new: 8, text: "after" },
  ];
  h.mod.render(h.cx);
  assert.equal(h.state.diffList.count, 1);
  const start = h.nodes.length;
  h.state.diffList.render(0);
  assert.ok(h.nodes.slice(start).every(n => Object.keys(n.events).length === 0));
  const frame = h.nodes.filter(n => n.id === "full-diff-frame").at(-1);
  frame.events.on_mouse_down({ bounds: { width: 1000 }, local_position: { x: 20 }, modifiers: {} });
  h.nodes.filter(n => n.id === h.state.diffList.id).at(-1).events.on_item_click("row-0", h.cx);
  assert.equal(h.mod.selection.side, "old"); assert.equal(h.mod.selection.start, 5);
  frame.events.on_mouse_down({ bounds: { width: 1000 }, local_position: { x: 750 }, modifiers: {} });
  h.nodes.filter(n => n.id === h.state.diffList.id).at(-1).events.on_item_click("row-0", h.cx);
  assert.equal(h.mod.selection.side, "new"); assert.equal(h.mod.selection.start, 8);
});

test("read toggles omit revision and unread filtering never changes send eligibility", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain(); h.mod.render(h.cx);
  h.nodes.find(n => n.id === "read-note1").events.on_click(null, h.cx); await h.drain();
  assert.equal(h.requests.at(-1).op, "read"); assert.equal(h.requests.at(-1).read, true);
  assert.equal("revision" in h.requests.at(-1), false);
  const items = [{ id: "a", read: false }, { id: "b", read: true }];
  assert.deepEqual(review.unreadItems(items, true), [items[0]]);
  assert.equal(review.pendingComments([{ read: true, resolved: false, stale: false }]).length, 1);
});

test("split rows pair unequal changes and never cross hunks", () => {
  const rows = [
    { kind: "delete", old: 1, new: null, text: "a" },
    { kind: "delete", old: 2, new: null, text: "b" },
    { kind: "add", old: null, new: 1, text: "c" },
    { kind: "hunk", text: "@@", old: null, new: null },
    { kind: "context", old: 20, new: 19, text: "same" },
  ];
  const split = review.splitRows(rows);
  assert.equal(split.length, 4);
  assert.equal(split[1].new, null);
  assert.equal(review.splitSelectionRow(split[1], "new"), null);
  assert.equal(review.splitSelectionRow(split[3], "old").new, null);
  assert.equal(review.splitSelectionRow(split[3], "new").old, null);
});

test("compact sidebar keeps comparison in the Files row with on-demand options", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain(); h.mod.render(h.cx);
  assert.ok(h.nodes.some(n => n.id === "files-scope-row"));
  assert.ok(!h.nodes.some(n => n.id === "scope-options"));
  assert.ok(!h.nodes.some(n => n.children.includes("Working tree")));
  h.nodes.find(n => n.id === "scope-menu").events.on_click(null, h.cx);
  h.mod.render(h.cx);
  h.nodes.filter(n => n.id === "scope-branch").at(-1).events.on_click(null, h.cx);
  await h.drain();
  assert.equal(h.requests.at(-1).scope.comparison.mode, "branch");
  assert.equal(h.mod.scopeMenuOpen, false);
});

test("full reader defaults to two columns with inline notes and an on-demand composer", async () => {
  const h = harness(); h.state.props = { presentation: "full" };
  h.mod.init({}, h.cx); await h.drain(); h.mod.render(h.cx);
  assert.equal(h.state.panelCalls, 2);
  assert.equal(h.mod.commentsOpen, false);
  assert.ok(!h.nodes.some(n => n.id === "full-composer"));
  assert.equal(h.state.diffList.count, 2, "one source row plus its inline note");
  const start = h.nodes.length;
  h.state.diffList.render(1);
  assert.ok(h.nodes.slice(start).some(n => n.id === "full-inline-note1"));
  assert.ok(h.nodes.slice(start).every(n => Object.keys(n.events).length === 0));
  h.nodes.filter(n => n.id === h.state.diffList.id).at(-1).events.on_item_click("note-note1", h.cx);
  h.mod.render(h.cx);
  assert.ok(h.nodes.some(n => n.id === "full-comment-actions"));
  h.nodes.filter(n => n.id === "full-new-file-comment").at(-1).events.on_click(null, h.cx);
  h.mod.render(h.cx);
  assert.ok(h.nodes.some(n => n.id === "full-composer"));
  h.nodes.filter(n => n.id === "full-toggle-comments").at(-1).events.on_click(null, h.cx);
  const before = h.state.panelCalls; h.mod.render(h.cx);
  assert.equal(h.state.panelCalls - before, 3);
  assert.equal(h.state.shared["annotator.fullComments"], true);
});

test("inline note placement respects diff sides, hidden context and file notes", () => {
  const rows = [{ kind: "delete", old: 5, new: null }, { kind: "add", old: null, new: 5 }];
  const comments = [
    { id: "file", position: null, stale: false },
    { id: "old", position: { side: "old", end: 5 } },
    { id: "new", position: { side: "new", end: 5 } },
    { id: "hidden", position: { side: "new", end: 100 } },
    { id: "stale", stale: true },
  ];
  assert.deepEqual(review.diffEntries(rows, comments, false).map(e => e.key),
    ["note-file", "row-0", "note-old", "row-1", "note-new", "note-hidden", "note-stale"]);
  assert.equal(review.diffEntries(review.splitRows(rows), comments, true).filter(e => e.comment).length, 5);
});

test("full review hides Changes on entry and restores it only on the source tab", () => {
  const panel = { id: "panel", stack: "panel-body", active: "changes", hidden: false, changes: true };
  const reviews = { review: { space: "space", sourceTab: "source", sourcePanel: panel } };
  const enter = review.reviewSidebarTransition(null, { tab: "review", space: "space" }, reviews, panel);
  assert.deepEqual(enter.actions, [{ name: "layout.hide", args: { id: "panel" } }]);
  assert.equal(review.reviewSidebarTransition(enter.state, { tab: "review", space: "space" }, reviews, panel), null,
    "manual reopening in the same review must not be immediately hidden again");
  const other = review.reviewSidebarTransition(enter.state, { tab: "other", space: "elsewhere" }, reviews, { ...panel, hidden: true });
  assert.deepEqual(other.actions, []);
  const source = review.reviewSidebarTransition(other.state, { tab: "source", space: "space" }, reviews, { ...panel, hidden: true });
  assert.deepEqual(source.actions.map(a => a.name), ["layout.select", "layout.show"]);
  const files = review.reviewSidebarTransition(enter.state, { tab: "source", space: "space" }, reviews, { ...panel, active: "files", changes: false });
  assert.deepEqual(files.actions, [], "do not replace a Files panel opened manually");
  const originallyHidden = { review: { ...reviews.review, sourcePanel: { ...panel, hidden: true } } };
  const hidden = review.reviewSidebarTransition(null, { tab: "review", space: "space" }, originallyHidden, { ...panel, hidden: true });
  assert.deepEqual(hidden.state.pending, {});
  const beforeRegistration = review.reviewSidebarTransition(null, { tab: "review", space: "space" }, {}, panel);
  assert.deepEqual(beforeRegistration.actions, []);
  const registered = review.reviewSidebarTransition(beforeRegistration.state, { tab: "review", space: "space" }, reviews, panel);
  assert.equal(registered.actions[0].name, "layout.hide", "registration may arrive after Herdr has already selected the new tab");
});

test("full review defaults to Branch and restores global preferences in a fresh tab", async () => {
  const h = harness(); h.state.props = { presentation: "full" };
  h.mod.init({}, h.cx); await h.drain();
  assert.equal(h.requests[0].scope.comparison.mode, "branch");
  h.mod.changeComparison("branch", "origin/custom-base", h.cx); await h.drain();
  h.mod.render(h.cx);
  for (const id of ["full-unified", "full-toggle-files", "full-toggle-comments", "full-unread"]) {
    h.nodes.filter(n => n.id === id).at(-1).events.on_click(null, h.cx);
  }
  h.mod.expandContext(23, h.cx); await h.drain();
  const next = harness();
  next.state.shared = JSON.parse(JSON.stringify(h.state.shared));
  next.state.props = { presentation: "full" };
  next.state.scope = { repo: "/another-checkout", machine: null };
  next.mod.init({}, next.cx); await next.drain();
  assert.equal(next.requests[0].scope.comparison.mode, "branch");
  assert.equal(next.requests[0].scope.comparison.base, null, "base refs must not leak to another checkout");
  assert.equal(next.requests[0].view.context, 23);
  assert.equal(next.mod.diffStyle, "unified");
  assert.equal(next.mod.filesOpen, false);
  assert.equal(next.mod.commentsOpen, true);
  assert.equal(next.mod.onlyUnread, true);
  const same = harness(); same.state.shared = JSON.parse(JSON.stringify(h.state.shared));
  same.state.props = { presentation: "full" };
  same.mod.init({}, same.cx); await same.drain();
  assert.equal(same.requests[0].scope.comparison.base, "origin/custom-base");
  next.mod.changeComparison("uncommitted", null, next.cx); await next.drain();
  const later = harness(); later.state.shared = JSON.parse(JSON.stringify(next.state.shared));
  later.state.props = { presentation: "full" };
  later.mod.init({}, later.cx); await later.drain();
  assert.equal(later.requests[0].scope.comparison.mode, "uncommitted");
});

test("malformed saved preferences use safe defaults", () => {
  const prefs = review.fullPreferences({ mode: "bad", contextLines: -2, filesOpen: "yes", diffStyle: "bad" });
  assert.equal(prefs.mode, "branch"); assert.equal(prefs.contextLines, 3);
  assert.equal(prefs.filesOpen, true); assert.equal(prefs.diffStyle, "split");
});

test("syntax toggles persist and colored rows keep the deferred renderer handler-free", async () => {
  const h = harness(); h.state.props = { presentation: "full" };
  h.mod.init({}, h.cx); await h.drain();
  const file = h.mod.data.snapshot.files[0];
  file.rows[0].text = 'const value = "hello";';
  h.mod.syntaxCache = null; h.mod.prepareSyntax(h.mod.data.snapshot);
  h.mod.render(h.cx);
  const start = h.nodes.length;
  h.state.diffList.render(0);
  assert.ok(h.nodes.slice(start).every(n => Object.keys(n.events).length === 0));
  assert.equal(h.mod.syntax, true);
  h.nodes.filter(n => n.id === "full-syntax").at(-1).events.on_click(null, h.cx);
  assert.equal(h.mod.syntax, false); assert.equal(h.mod.syntaxCache, null);
  const next = harness(); next.state.shared = JSON.parse(JSON.stringify(h.state.shared));
  next.state.props = { presentation: "full" };
  next.mod.init({}, next.cx); await next.drain();
  assert.equal(next.mod.syntax, false);
});

test("unreachable machines show a connection state, not an empty checkout", async () => {
  const h = harness(); h.state.scope.machine = "manjaro";
  const error = 'Command failed (5): {"error":{"kind":"unavailable","message":"manjaro is unreachable: cannot reach manjaro: ssh: connect to host 192.0.2.10 port 22: Operation timed out","hint":"hc machine ls shows when it comes back"}}';
  h.state.reply = async () => ({ code: 1, stdout: JSON.stringify({ ok: false, error }) });
  h.mod.init({}, h.cx); await h.drain(); h.mod.render(h.cx);
  assert.equal(h.mod.snapshotError.kind, "offline");
  const text = h.nodes.flatMap(n => n.children).filter(v => typeof v === "string");
  assert.ok(text.includes("manjaro is unavailable"));
  assert.ok(!text.includes("Select a Git checkout"));
  assert.ok(text.some(v => v.includes("Files · —")));
  assert.ok(!text.some(v => v.includes('"kind":"unavailable"')));
});

test("failed refresh preserves cached rows but blocks review actions until recovery", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  const cached = h.mod.data;
  h.state.reply = async () => ({ code: 1, stdout: JSON.stringify({ ok: false, error: "Connection timed out" }) });
  h.mod.refresh(h.cx); await h.drain();
  assert.equal(h.mod.data, cached);
  const count = h.requests.length;
  h.mod.sendToSession(h.cx);
  h.mod.act("resolve", { id: "note1", resolved: true }, h.cx);
  await h.drain(); assert.equal(h.requests.length, count);
  h.mod.render(h.cx);
  assert.ok(!h.nodes.filter(n => n.id === "send-review").at(-1).events.on_click);
  h.state.reply = null;
  h.mod.refresh(h.cx, true); await h.drain();
  assert.equal(h.mod.snapshotError, null);
  h.mod.render(h.cx);
  assert.ok(h.nodes.filter(n => n.id === "send-review").at(-1).events.on_click);
});

test("known offline machines skip automatic Git calls but Retry still probes", async () => {
  const h = harness(); h.state.scope.machine = "manjaro";
  h.state.machines = [{ name: "manjaro", layout: "unavailable" }];
  h.mod.init({}, h.cx); await h.drain();
  assert.equal(h.requests.length, 0);
  assert.equal(h.mod.snapshotError.kind, "offline");
  h.mod.refresh(h.cx, true); await h.drain();
  assert.equal(h.requests[0].op, "snapshot");
});

test("retry after disconnection preserves drafts and requires reselection on changed code", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.mod.editor.set_value("keep this draft");
  h.mod.snapshotFailed("Connection timed out", h.state.scope);
  h.state.revision = "changed-while-offline";
  h.mod.refresh(h.cx, true); await h.drain();
  assert.equal(h.mod.editor.value(), "keep this draft");
  assert.equal(h.mod.needsSelection, true);
  assert.equal(h.mod.snapshotError, null);
});

test("confirmed Send focuses only the exact recipient; failed Send never changes focus", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.mod.sendToSession(h.cx); await h.drain();
  assert.equal(h.state.commands.at(-1)?.name, "pane.focus");
  assert.equal(h.state.commands.at(-1)?.args.terminal, "t7");
  h.state.commands = [];
  h.state.reply = async () => ({ code: 1, stdout: JSON.stringify({ ok: false, error: "Delivery not confirmed" }) });
  h.mod.sendToSession(h.cx); await h.drain();
  assert.equal(h.state.commands.length, 0);
});

test("Cmd/Ctrl Enter saves one draft without sending; plain Enter stays multiline", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.mod.editor.set_value("save by shortcut"); h.mod.editor.focus();
  h.mod.handleKey({ key: "enter", keystroke: "enter", modifiers: {} }, h.cx);
  await h.drain(); assert.equal(h.requests.filter(r => r.op === "add").length, 0);
  h.mod.handleKey({ key: "enter", keystroke: "cmd-enter", modifiers: { platform: true } }, h.cx);
  h.mod.editor.emit("submit", { secondary: true });
  await h.drain();
  assert.equal(h.requests.filter(r => r.op === "add").length, 1);
  assert.equal(h.requests.filter(r => r.op === "send").length, 0);
  assert.equal(h.mod.editor.value(), "");
  h.mod.editor.set_value("offline draft"); h.mod.snapshotFailed("Connection timed out", h.state.scope);
  h.mod.editor.emit("submit", { secondary: true }); await h.drain();
  assert.equal(h.requests.filter(r => r.op === "add").length, 1);
  assert.equal(h.mod.editor.value(), "offline draft");
});

test("legacy origin/develop becomes Auto; explicit remote choices remain pinned", async () => {
  const h = harness(); h.state.props = { presentation: "full" };
  const key = JSON.stringify([null, "/checkout"]);
  h.state.shared["annotator.baseRefs"] = { [key]: "origin/develop" };
  h.state.reply = async request => {
    const data = h.snapshot();
    if (request.op === "snapshot") data.snapshot.scope.comparison = { mode: "branch", base: request.scope.comparison.base ?? "develop" };
    return { code: 0, stdout: JSON.stringify({ ok: true, data }) };
  };
  h.mod.init({}, h.cx); await h.drain();
  assert.equal(h.requests[0].scope.comparison.base, null);
  assert.equal(h.mod.base, "develop"); assert.equal(h.mod.baseExplicit, false);
  assert.equal(h.state.shared["annotator.baseRefs"][key].explicit, false);
  h.mod.changeComparison("branch", "origin/develop", h.cx, true); await h.drain();
  assert.equal(h.requests.at(-1).scope.comparison.base, "origin/develop");
  const next = harness(); next.state.props = { presentation: "full" };
  next.state.shared = JSON.parse(JSON.stringify(h.state.shared));
  next.mod.init({}, next.cx); await next.drain();
  assert.equal(next.requests[0].scope.comparison.base, "origin/develop");
  next.mod.changeComparison("branch", null, next.cx, false); await next.drain();
  assert.equal(next.requests.at(-1).scope.comparison.base, null);
  assert.equal(next.state.shared["annotator.baseRefs"][key].explicit, false);
});

test("save shortcut requires composer focus; confirmed Send never redirects to a replacement chat", async () => {
  const h = harness(); h.mod.init({}, h.cx); await h.drain();
  h.mod.editor.set_value("draft");
  h.mod.handleKey({ key: "enter", keystroke: "ctrl-enter", modifiers: { control: true } }, h.cx);
  await h.drain(); assert.equal(h.requests.filter(r => r.op === "add").length, 0);
  h.mod.editor.focus();
  h.mod.handleKey({ key: "enter", keystroke: "ctrl-enter", modifiers: { control: true } }, h.cx);
  await h.drain(); assert.equal(h.requests.filter(r => r.op === "add").length, 1);
  h.state.reply = async () => {
    h.state.agents = [{ address: "t8", pane: "p8", space: "space1", tab: "tab1", status: "idle" }];
    return { code: 0, stdout: JSON.stringify({ ok: true, data: h.snapshot() }) };
  };
  h.mod.sendToSession(h.cx); await h.drain();
  assert.equal(h.state.commands.length, 0);
});
