import test from "node:test";
import assert from "node:assert/strict";
import { GithubRefresh } from "../mods/annotator/github-refresh.js";

test("replacement checkout demand survives old completion and retains force", async () => {
  const refresh = new GithubRefresh(), calls = [];
  let release, current = "old";
  const request = async work => {
    calls.push(work);
    if (work.scope.repo === "old") await new Promise(resolve => { release = resolve; });
    return { github: { threads: [], repo: work.scope.repo } };
  };
  refresh.demand("old", { repo: "old" });
  const job = refresh.drain(request, key => key === current, () => {});
  refresh.invalidate(); current = "new";
  refresh.demand("new", { repo: "new" }, true);
  refresh.demand("new", { repo: "new" });
  await refresh.drain(request, key => key === current, () => {});
  release(); await job;
  assert.equal(calls.length, 2); assert.equal(calls[1].force, true);
  assert.equal(refresh.value.repo, "new"); assert.equal(refresh.busy, false);
});

test("branch invalidation rejects old results; forced refresh and queue reconcile follow", async () => {
  const refresh = new GithubRefresh(); let release;
  refresh.demand("same", { repo: "checkout" });
  let calls = 0;
  const job = refresh.drain(async () => {
    if (++calls === 1) await new Promise(resolve => { release = resolve; });
    return { github: { threads: [{ id: "keep", fingerprint: "v2" }] } };
  }, () => true, () => {});
  refresh.invalidate();
  refresh.queue.set("keep", { fingerprint: "v2" });
  refresh.queue.set("stale", { fingerprint: "v1" });
  refresh.demand("same", { repo: "checkout" }, true);
  release(); await job;
  assert.equal(calls, 2); assert.equal(refresh.queue.size, 1);
  assert.equal(refresh.queue.has("keep"), true);
});

test("branch invalidation during snapshot refresh retains queued force intent", async () => {
  const refresh = new GithubRefresh(), calls = []; let release;
  refresh.demand("same", { repo: "checkout" });
  const job = refresh.drain(async work => {
    calls.push(work);
    if (calls.length === 1) await new Promise(resolve => { release = resolve; });
    return { github: { threads: [] } };
  }, () => true, () => {});
  refresh.demand("same", { repo: "checkout" }, true);
  // Snapshot discovers the changed branch after manual Refresh queued force.
  refresh.invalidate(); refresh.demand("same", { repo: "checkout" });
  release(); await job;
  assert.equal(calls.length, 2); assert.equal(calls[1].force, true);
});
