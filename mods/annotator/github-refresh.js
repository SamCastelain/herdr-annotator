// Request ownership only. Cache freshness and locking remain in Rust.
export class GithubRefresh {
  constructor() {
    this.queue = new Map(); this.generation = 0; this.busy = false;
    this.invalidate();
  }

  invalidate() {
    this.carryForce = Boolean(this.carryForce || this.pending?.force || this.active?.force);
    this.generation++; this.value = null; this.context = null;
    this.pending = null; this.queue.clear();
  }

  demand(context, scope, force = false) {
    this.pending = { context, scope, generation: this.generation,
      force: force || this.carryForce || Boolean(this.pending?.context === context && this.pending.force) };
    this.carryForce = false;
  }

  async drain(request, isCurrent, changed) {
    if (this.busy) return;
    this.busy = true;
    try {
      while (this.pending) {
        const work = this.pending; this.pending = null;
        const current = () => work.generation === this.generation && isCurrent(work.context);
        if (!current()) continue;
        this.active = work;
        try {
          const data = await request({ op: "github", scope: work.scope, force: work.force });
          if (!current()) continue;
          this.value = data.github; this.context = work.context;
          for (const [id, ref] of this.queue) {
            if (!this.value?.threads?.some(t => t.id === id && t.fingerprint === ref.fingerprint
              && !t.resolved && !t.delivery && !t.incomplete)) this.queue.delete(id);
          }
        } catch (e) {
          if (current()) this.value = { ...this.value, error: String(e) };
        } finally { this.active = null; }
        changed();
      }
    } finally { this.busy = false; changed(); }
  }
}
