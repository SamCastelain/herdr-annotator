import test from "node:test";
import assert from "node:assert/strict";
import { UpdateController } from "../mods/annotator/updates.js";

const release = { tag: "v0.3.0", version: "0.3.0", commit: "a".repeat(40), url: "https://github.com/jakub-tucek/herdr-annotator/releases/tag/v0.3.0" };

function harness() {
  const shared = {}, storage = new Map(), jobs = [], timers = [], calls = [];
  const state = { response: null, worker: false };
  const env = {
    script: "/install/update.py", appHome: "/app", shared: key => shared[key], setShared: (key, value) => { shared[key] = value; },
    storage: { getItem: key => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) },
    exec: async (binary, argv) => {
      calls.push({ binary, argv });
      if (state.response) return state.response(binary, argv);
      const command = argv[1];
      if (command === "start") state.worker = true;
      const data = command === "check" ? { current: "0.2.0", latest: release, checked_at: Date.now() / 1000, job: {} }
        : command === "status" && !state.worker ? {}
        : { id: "job1", status: "running", tag: release.tag };
      return { code: 0, stdout: JSON.stringify({ ok: true, data }) };
    },
  };
  const cx = { notify() {}, spawn(fn) { jobs.push(Promise.resolve().then(() => fn(cx))); }, timer: {
    every(ms, tick) { const timer = { ms, tick, cancelled: false, cancel() { this.cancelled = true; } }; timers.push(timer); return timer; },
  } };
  return { env, cx, state, calls, timers, make: () => new UpdateController(env), drain: async () => { while (jobs.length) await Promise.all(jobs.splice(0)); } };
}

test("automatic checks show a release but never start an install; daily throttle survives copies", async () => {
  const h = harness(), view = h.make(); view.publish(false); view.init(h.cx); await h.drain();
  assert.equal(view.state.latest.tag, release.tag);
  assert.deepEqual(h.calls.map(c => c.argv[1]), ["status", "check"]);
  assert.equal(view.running, false);
  const copy = h.make(); copy.init(h.cx); await h.drain();
  assert.equal(h.calls.length, 2);
  copy.check(h.cx, true); await h.drain(); assert.ok(h.calls.at(-1).argv.includes("--force"));
});

test("hidden review blockers do not expire and block explicit Update", async () => {
  const h = harness(), view = h.make(), hidden = h.make();
  view.publish(false); hidden.publish(true); view.save({ latest: release });
  view.start(h.cx); await h.drain(); assert.equal(h.calls.length, 0); assert.equal(view.running, false);
  assert.match(view.state.error, /all review views/);
  hidden.close(); view.start(h.cx); await h.drain();
  assert.equal(h.calls.filter(c => c.argv[1] === "start").length, 1);
  assert.equal(view.running, true); assert.equal(hidden.running, true);
});

test("a draft appearing during the click aborts before launching a worker", async () => {
  const h = harness(), view = h.make(); view.publish(false); view.save({ latest: release });
  view.start(h.cx); view.publish(true); await h.drain();
  assert.equal(h.calls.length, 0); assert.equal(view.running, false);
  assert.match(view.state.error, /postponed/);
});

test("worker failures restore editors; failed status polling keeps the gate closed", async () => {
  const h = harness(), view = h.make(); view.publish(false); view.save({ latest: release });
  h.state.response = async (_binary, argv) => {
    if (argv[1] === "status") throw new Error("read failed");
    return { code: 0, stdout: JSON.stringify({ ok: true, data: { status: "running", id: "j1" } }) };
  };
  view.start(h.cx); await h.drain(); assert.equal(view.running, true);
  assert.match(view.state.error, /gate remains closed/);
  h.state.response = async () => ({ code: 0, stdout: JSON.stringify({ ok: true, data: { status: "failed", error: "dirty checkout" } }) });
  h.timers.find(t => t.ms === 2000).tick(h.cx); await h.drain();
  assert.equal(view.running, false); assert.equal(view.state.error, "dirty checkout");
});

test("reload resumes a worker instead of opening editors or launching another install", async () => {
  const h = harness(), first = h.make(); first.publish(false); first.save({ latest: release });
  first.start(h.cx); await h.drain(); first.close();
  const next = h.make(); next.init(h.cx); await h.drain();
  assert.equal(next.running, true); assert.equal(h.calls.filter(c => c.argv[1] === "start").length, 1);
  next.close(); assert.ok(h.timers.every(t => t.cancelled));
});

test("recorded Python interpreter is invoked with literal paths", async () => {
  const h = harness(); h.env.python = "C:\\Python (中文)\\python.exe";
  const view = h.make(); await view.command("status");
  assert.equal(h.calls[0].binary, h.env.python);
  assert.deepEqual(h.calls[0].argv, [h.env.script, "status", "--app-home", h.env.appHome]);
});

test("unsupported Windows updater never starts a process or blocks review views", async () => {
  const h = harness(); h.env.updatesEnabled = false;
  h.env.storage.setItem("annotator.updateGate", "true");
  const view = h.make(); view.publish(false); view.init(h.cx);
  view.check(h.cx, true); view.start(h.cx); view.poll(h.cx); await h.drain();
  assert.equal(view.running, false);
  assert.equal(h.calls.length, 0); assert.equal(h.timers.length, 0);
  assert.match(view.state.error, /not supported on Windows/);
});
