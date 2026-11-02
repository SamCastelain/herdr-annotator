import { View, div } from "gpui-kit";
import { v_flex, h_flex, v_resizable, resizable_panel, v_virtual_list, Scrollbar, TextareaState, Textarea } from "gpui-base";
import { spaces, panes, focused, agents, exec, home, props, theme, settings, shared, setShared, run, layout, appHome, watchFolder, machines } from "herdr";
import { selectionFor, selectionPayload, selected, inlineAt, pendingComments, sessionTarget, fileStats, pathParts, commentStatus, commentLocation, reviewSummary, sendHint, unreadItems, layoutSidebar, reviewSidebarTransition, fullPreferences, snapshotFailure, needsFreshSnapshot, savedBaseChoice } from "./review.js";
import { renderFull } from "./full.js";
import { highlightDiff } from "./syntax.js";
import { DraftLifecycle } from "./draft.js";

function location() {
  const fixed = props()?.scope;
  if (fixed) return { scope: fixed, space: props()?.space ?? null, tab: props()?.tab ?? null };
  const f = focused();
  const space = spaces().find(s => s.id === f?.space) ?? spaces().find(s => s.active);
  const tabs = new Set((space?.tabs ?? []).filter(t => t.active).map(t => t.id));
  const allTabs = new Set((space?.tabs ?? []).map(t => t.id));
  const ps = panes();
  if (f?.pane === null && f?.terminal && f?.cwd) {
    const machine = f.terminal.includes("/") ? f.terminal.split("/")[0] : null;
    const owner = agents().find(a => a.address === f.terminal);
    return { scope: { repo: f.cwd, machine }, space: owner?.space ?? null, tab: owner?.tab ?? f.tab ?? null };
  }
  const pane = ps.find(p => p.pane === f?.pane && p.terminal) ?? ps.find(p => tabs.has(p.tab) && p.cwd) ?? ps.find(p => allTabs.has(p.tab) && p.cwd);
  return { scope: { repo: pane?.cwd || space?.folder || "", machine: space?.machine ?? null }, space: space?.id ?? null, tab: f?.tab ?? space?.tabs?.find(t => t.active)?.id ?? null };
}
const contextKey = () => JSON.stringify(location());

export default class Annotator extends View {
  init(_props, cx) {
    this.data = null; this.open = null; this.busy = false;
    this.error = null; this.notice = null; this.context = null;
    this.snapshotError = null; this.showErrorDetails = false;
    this.sessions = new Map(); this.exportText = null;
    this.full = props()?.presentation === "full"; this.onlyUnread = false;
    const preferences = this.fullSettings();
    this.commentsOpen = preferences.commentsOpen;
    this.diffStyle = preferences.diffStyle;
    this.syntax = preferences.syntax; this.syntaxCache = null;
    this.onlyUnread = this.full && preferences.onlyUnread;
    const widths = shared("annotator.fullWidths");
    this.fullWidths = Array.isArray(widths) && [2, 3].includes(widths.length) && widths.every(n => Number.isFinite(n) && n > 0)
      ? [widths[0], widths.slice(1).reduce((sum, n) => sum + n, 0)] : [260, 1230];
    this.commentWidth = Array.isArray(widths) && widths.length === 3 ? widths[2] : 330;
    this.mode = "uncommitted"; this.base = null; this.contextLines = 3; this.baseOpen = false; this.scopeMenuOpen = false;
    this.baseExplicit = false; this.editorFocused = false;
    this.github = null; this.githubBusy = false; this.threadQueue = new Map(); this.bases = []; this.githubEpoch = 0;
    const sizes = shared("annotator.sections");
    this.sizes = Array.isArray(sizes) && sizes.length === 3 && sizes.every(n => Number.isFinite(n) && n > 0) ? sizes : [140, 360, 175];
    this.view = ["comments", "github"].includes(props()?.view) ? props().view : "diff"; this.filesOpen = this.full ? preferences.filesOpen : true;
    this.editor = TextareaState.new({ placeholder: "What should change? Ask a question or leave feedback…", rows: 3 });
    this.draft = new DraftLifecycle(this.editor);
    this.editor.on("change", (_e, cx) => cx.notify());
    this.editor.on("focus", () => { this.editorFocused = true; });
    this.editor.on("blur", () => { this.editorFocused = false; });
    this.editor.on("submit", (event, cx) => { if (event.secondary && !event.shift) this.saveComment(cx); });
    this.output = TextareaState.new({ placeholder: "Exported review", rows: 6 });
    this.handle = cx.focus_handle();
    this.refresh(cx);
    cx.timer.every(15000, cx => { if (!this.editor.value().trim()) this.refresh(cx); });
    cx.timer.every(300000, cx => this.refreshGithub(cx));
    // A hidden sidebar and an inactive review tab need not render. Herdr saves
    // focus in view.json; native file events keep restoration alive without
    // polling focus or keeping a redraw timer alive.
    cx.spawn(async cx => {
      try {
        for (;;) { await watchFolder(appHome()); this.syncSidebar(); cx.notify(); }
      } catch (e) { this.sidebarWatchError = String(e); }
    });
  }

  helper() { return props()?.helper ?? `${home()}/.local/bin/herdr-annotator`; }

  get selection() { return this.draft.selection; }
  set selection(value) { this.draft.selection = value; }
  get needsSelection() { return this.draft.needsSelection; }
  get composerOpen() { return this.draft.composerOpen; }
  set composerOpen(value) { this.draft.composerOpen = value; }
  get drafts() { return this.draft.saved; }

  fileNote(cx) {
    this.draft.anchor(null, this.data?.snapshot.revision);
    this.activeComment = null; this.editor.focus(); cx.notify();
  }

  cancelDraft(cx) {
    this.draft.cancel(this.context); this.notice = null; cx.notify();
  }

  fullSettings() {
    return fullPreferences(shared("annotator.fullPreferences"), {
      diffStyle: shared("annotator.diffStyle"), commentsOpen: shared("annotator.fullComments"),
    });
  }

  rememberFull(patch) {
    if (!this.full) return;
    setShared("annotator.fullPreferences", { ...this.fullSettings(), ...patch });
  }

  rememberBase(scope, base) {
    if (!this.full || !base) return;
    const key = JSON.stringify([scope.machine ?? null, scope.repo]);
    const bases = shared("annotator.baseRefs") ?? {};
    const entry = { base, explicit: this.baseExplicit };
    if (bases[key]?.base !== base || bases[key]?.explicit !== entry.explicit) setShared("annotator.baseRefs", { ...bases, [key]: entry });
  }

  baseLabel() { return this.baseExplicit ? this.base ?? "Base…" : this.base ? `Auto: ${this.base}` : "Auto"; }

  saveComment(cx) {
    if (this.busy || this.snapshotError || !this.data || this.needsSelection || !this.editor.value().trim()
      || !this.data.snapshot.files.some(file => file.path === this.open)) return;
    this.act("add", { path: this.open, selection: selectionPayload(this.selection), body: this.editor.value() }, cx);
  }

  handleKey(event, cx) {
    const chord = ["cmd-enter", "ctrl-enter", "cmd-return", "ctrl-return"].includes(event.keystroke);
    if (chord && this.editorFocused && !event.is_held && !event.modifiers?.shift && !event.modifiers?.alt) {
      cx.stop_propagation(); this.saveComment(cx); return;
    }
    if (event.key === "escape") { this.requeue = null; this.exportText = null; cx.notify(); }
  }

  prepareSyntax(snapshot) {
    if (!this.syntax || !snapshot) { this.syntaxCache = null; return; }
    const file = snapshot.files.find(f => f.path === this.open);
    if (!file || (!this.full && file.rows.length > 120)) { this.syntaxCache = null; return; }
    const key = JSON.stringify([snapshot.revision, file.path]);
    if (this.syntaxCache?.key === key) return;
    const tokens = highlightDiff(file.rows, file.path);
    // The compact sidebar builds its code eagerly. Bound its native text-run
    // count; full review virtualizes rows and can color the whole preview.
    const count = [...tokens.values()].reduce((sum, line) => sum + line.length, 0);
    this.syntaxCache = { key, tokens: !this.full && count > 600 ? new Map() : tokens };
  }

  codeText(row, side, t, highlights) {
    const source = side ?? (row.new != null ? "new" : "old");
    const tokens = this.syntax ? highlights?.get(`${source}:${row[source]}`) : null;
    if (!tokens?.some(token => token.kind !== "plain")) return div().flex_1().min_w(0).child(row.text || " ");
    const colors = { keyword: t.ansi5, string: t.ansi2, comment: t.muted,
      number: t.ansi3, literal: t.ansi3, function: t.ansi4, type: t.ansi6, property: t.ansi4, plain: t.foreground };
    return h_flex().flex_1().min_w(0).flex_wrap().items_start().gap(0)
      .children(tokens.map(token => div().max_w("100%").text_color(colors[token.kind] ?? t.foreground).child(token.text)));
  }

  async request(request) {
    // This is always local. The Rust engine uses hc to read remote repos.
    const result = await exec(this.helper(), ["request", JSON.stringify(request)]);
    let response;
    try { response = JSON.parse(result.stdout); }
    catch { throw new Error(result.stderr || "Helper returned no JSON; build/install herdr-annotator first"); }
    if (result.code !== 0 || !response.ok) throw new Error(response.error ?? result.stderr ?? "Review operation failed");
    return response.data;
  }

  machineUnavailable(scope) {
    return Boolean(scope.machine && machines().some(machine => machine.name === scope.machine && machine.layout === "unavailable"));
  }

  snapshotFailed(error, scope) {
    this.snapshotError = snapshotFailure(error, scope.machine);
    this.error = this.snapshotError.message;
  }

  errorCard(t) {
    const failure = this.snapshotError;
    return v_flex().id("review-error").m(8).p(10).gap(6).rounded(6).bg(t.ansi1 + "12").text_size(12)
      .child(div().font_semibold().text_color(t.ansi1).child(failure?.title ?? "Review operation failed"))
      .child(div().text_color(t.muted).child(this.error))
      .when(Boolean(failure && this.data), el => el.child(div().text_color(t.ansi3).child("Showing the last loaded diff. Review actions are paused until Refresh succeeds.")))
      .when(Boolean(failure), el => el.child(h_flex().gap(5)
        .child(this.button("error-retry", "↻ Retry", cx => this.refresh(cx, true), true, "secondary"))
        .when(failure.kind === "offline", el => el.child(this.button("error-machines", "Machines…", () => run("app.settings", { section: "machines" }))))
        .when(Boolean(failure.detail), el => el.child(this.button("error-details", this.showErrorDetails ? "Hide details" : "Details", cx => { this.showErrorDetails = !this.showErrorDetails; cx.notify(); })))))
      .when(Boolean(this.showErrorDetails && failure?.detail), el => el.child(div().text_size(11).text_color(t.dim).child(failure.detail)));
  }

  refresh(cx, force = false) {
    if (this.busy) return;
    const next = contextKey();
    if (next !== this.context) {
      const draft = this.draft.enter(next, this.context, { open: this.open, mode: this.mode, base: this.base, baseExplicit: this.baseExplicit, contextLines: this.contextLines }, this.data?.snapshot.revision);
      this.context = next; this.data = null; this.open = draft?.open ?? props()?.file ?? null;
      const preferences = this.fullSettings(), scope = location().scope;
      const savedBase = (shared("annotator.baseRefs") ?? {})[JSON.stringify([scope.machine ?? null, scope.repo])];
      const propBase = props()?.scope?.comparison?.base;
      const propChoice = props()?.base_explicit === undefined ? propBase : { base: propBase, explicit: props().base_explicit };
      const choice = this.full ? savedBaseChoice(props()?.base_explicit === true ? propChoice : savedBase ?? propChoice)
        : { base: propBase ?? null, explicit: props()?.base_explicit ?? Boolean(propBase) };
      this.mode = draft?.mode ?? (this.full ? preferences.mode : props()?.scope?.comparison?.mode ?? "uncommitted");
      this.base = draft?.base ?? choice.base; this.baseExplicit = draft?.baseExplicit ?? choice.explicit;
      this.contextLines = draft?.contextLines ?? (this.full ? preferences.contextLines : props()?.context_lines ?? 3); this.bases = [];
      this.exportText = null; this.error = null; this.snapshotError = null; this.showErrorDetails = false; this.notice = null; this.activeComment = null; this.composerOpen = false;
      this.github = null; this.threadQueue.clear(); this.githubContext = null; this.githubEpoch++;
      this.view = ["comments", "github"].includes(props()?.view) ? props().view : "diff";
    } else if (this.editor.value().trim()) {
      if (!(force && this.snapshotError)) { this.notice = "Save or cancel your draft before refreshing."; cx.notify(); return; }
    }
    const current = location();
    if (!current.scope.repo) { this.error = "Select a space with a Git checkout."; return; }
    if (!force && this.machineUnavailable(current.scope)) {
      this.snapshotFailed(`Machine ${current.scope.machine} is unavailable`, current.scope); cx.notify(); return;
    }
    const context = this.context;
    this.busy = true;
    cx.spawn(async cx => {
      try {
        const data = await this.request({ op: "snapshot", scope: { ...current.scope, comparison: { mode: this.mode, base: this.baseExplicit ? this.base : null } }, view: { file: this.open, context: this.contextLines } });
        if (contextKey() !== context) return;
        const previousRevision = this.data?.snapshot.revision;
        if (this.data && this.data.snapshot.branch !== data.snapshot.branch) {
          this.github = null; this.githubContext = null; this.threadQueue.clear(); this.githubEpoch++;
        }
        this.data = data;
        this.bases = data.snapshot.bases ?? [];
        this.base = data.snapshot.scope.comparison?.base ?? this.base;
        if (this.mode === "branch") this.rememberBase(data.snapshot.scope, this.base);
        if (this.draft.observe(data.snapshot.revision, previousRevision)) {
          this.notice = "Diff changed since this draft. Select the intended lines again, or choose File note. Your text is kept.";
        }
        if (!this.open || (!this.editor.value().trim() && !data.snapshot.files.some(f => f.path === this.open))) {
          this.open = data.snapshot.files[0]?.path ?? null;
        }
        this.prepareSyntax(data.snapshot);
        this.error = null;
        this.snapshotError = null; this.showErrorDetails = false;
        if (this.githubContext !== context) this.refreshGithub(cx);
      } catch (e) { if (contextKey() === context) this.snapshotFailed(e, current.scope); }
      finally { this.busy = false; cx.notify(); }
    });
  }

  act(op, details, cx) {
    if (this.busy || this.snapshotError || !this.data || contextKey() !== this.context) return;
    const context = this.context;
    const snapshot = this.data.snapshot;
    const draftToken = op === "add" ? this.draft.capture(context) : null;
    this.busy = true; this.error = null; this.notice = null;
    cx.spawn(async cx => {
      try {
        const data = await this.request({ op, scope: snapshot.scope, view: snapshot.view ?? { file: this.open, context: this.contextLines }, ...(["resolve", "read"].includes(op) ? {} : { revision: snapshot.revision }), ...details });
        if (contextKey() !== context) {
          if (draftToken) this.draft.complete(draftToken, this.context);
          return;
        }
        if (op === "export") { this.exportText = data.text; this.output.set_value(data.text); }
        else { this.data = data; this.prepareSyntax(data.snapshot); }
        if (draftToken) { this.draft.complete(draftToken, this.context); this.activeComment = null; this.notice = "Comment saved. It is ready to send with your review."; }
        if (contextKey() !== context) return;
        if (op === "send") {
          this.notice = `Feedback submitted to the open session (${details.target}). Sent notes remain in this review.`;
          if (data.github) this.github = data.github;
          this.threadQueue.clear();
          if (agents().some(agent => agent.address === details.target)) run("pane.focus", { terminal: details.target });
        }
      } catch (e) { if (contextKey() === context) this.error = String(e); }
      finally { this.busy = false; cx.notify(); }
    });
    cx.notify();
  }

  currentSession() {
    const current = location();
    const pinned = props()?.source_target;
    if (pinned) return sessionTarget(agents().filter(a => a.address === pinned), current, null, pinned);
    const key = JSON.stringify([current.scope.machine, current.space, current.tab]);
    const target = sessionTarget(agents(), current, focused(), this.sessions.get(key));
    if (target) this.sessions.set(key, target.address);
    return target;
  }

  sendToSession(cx, thread = null) {
    if (this.busy || this.snapshotError || !this.data || contextKey() !== this.context || this.editor.value().trim()) return;
    const threads = thread ? [{ id: thread.id, fingerprint: thread.fingerprint }] : [...this.threadQueue.values()];
    if (!thread && !pendingComments(this.data.comments).length && !threads.length) return;
    const target = this.currentSession();
    if (!target || target.status !== "idle") {
      this.notice = target ? `The open session is ${target.status}. Send when it is idle; permission prompts are never bypassed.`
        : "Focus the agent chat in this tab first. Reviews are never routed to another tab automatically.";
      cx.notify(); return;
    }
    this.act("send", { target: target.address, threads, include_local: !thread }, cx);
  }

  refreshGithub(cx, force = false) {
    const context = contextKey(), scope = location().scope, epoch = this.githubEpoch;
    if (this.githubBusy || this.snapshotError || !scope.repo) return;
    this.githubBusy = true;
    cx.spawn(async cx => {
      try {
        const data = await this.request({ op: "github", scope, force });
        if (contextKey() !== context || epoch !== this.githubEpoch) return;
        this.github = data.github; this.githubContext = context;
        for (const [id, ref] of this.threadQueue) {
          if (!this.github?.threads?.some(t => t.id === id && t.fingerprint === ref.fingerprint && !t.resolved && !t.delivery && !t.incomplete)) this.threadQueue.delete(id);
        }
      } catch (e) { if (contextKey() === context && epoch === this.githubEpoch) this.github = { ...this.github, error: String(e) }; }
      finally { this.githubBusy = false; cx.notify(); }
    });
  }

  changeComparison(mode, base, cx, explicit = undefined) {
    if (this.busy || this.editor.value().trim()) { this.notice = "Save or cancel your draft before changing comparison."; cx.notify(); return; }
    this.baseExplicit = explicit ?? (base !== this.base ? Boolean(base) : this.baseExplicit);
    this.mode = mode; this.base = base; this.baseOpen = false; this.scopeMenuOpen = false; this.selection = null;
    this.rememberFull({ mode });
    if (mode === "branch") this.rememberBase(location().scope, base);
    if (mode === "branch" && !this.baseExplicit && !base && this.full) {
      const scope = location().scope, key = JSON.stringify([scope.machine ?? null, scope.repo]);
      setShared("annotator.baseRefs", { ...(shared("annotator.baseRefs") ?? {}), [key]: { base: null, explicit: false } });
    }
    this.activeComment = null; this.composerOpen = false;
    this.data = null;
    this.refresh(cx);
  }

  expandContext(lines, cx) {
    if (this.busy || this.editor.value().trim()) { this.notice = "Save or cancel your draft before changing context."; cx.notify(); return; }
    this.contextLines = lines; this.rememberFull({ contextLines: lines }); this.selection = null; this.refresh(cx);
  }

  openFull(cx) {
    if (!this.data || this.busy || this.openingFull) return;
    if (this.editor.value().trim()) { this.notice = "Save or cancel your draft before opening full review."; cx.notify(); return; }
    const current = location(), snapshot = this.data.snapshot, target = this.currentSession();
    const args = { name: `Review · ${snapshot.branch ?? "changes"}`, space: current.space,
      mod: "annotator", props: { presentation: "full", scope: snapshot.scope,
        space: current.space, tab: current.tab, source_target: target?.address ?? null,
        file: this.open, base_explicit: this.baseExplicit, context_lines: this.contextLines, view: this.view,
        ...(props()?.helper ? { helper: props().helper } : {}) } };
    const sourcePanel = layoutSidebar(layout());
    this.openingFull = true;
    cx.spawn(async cx => {
      try {
        // run() has no return value. The local app CLI returns the exact created
        // tab/pane, so only that tab is registered and focused, including on SSH.
        const result = await exec("herdr-app", ["do", "tab.new", JSON.stringify(args)]);
        const made = JSON.parse(result.stdout);
        if (result.code !== 0 || made.error || !made.tab || !made.pane) throw new Error(made.error ?? "Could not create review tab");
        const qualify = id => current.scope.machine && !id.startsWith(`${current.scope.machine}/`) ? `${current.scope.machine}/${id}` : id;
        const tab = qualify(made.tab), pane = qualify(made.pane);
        setShared("annotator.fullTabs", { ...(shared("annotator.fullTabs") ?? {}),
          [tab]: { sourceTab: current.tab, space: current.space, sourcePanel } });
        run("pane.focus", { pane });
      } catch (e) { this.error = String(e); }
      finally { this.openingFull = false; cx.notify(); }
    });
  }

  syncSidebar() {
    const transition = reviewSidebarTransition(shared("annotator.sidebarFocus"), focused() ?? {},
      shared("annotator.fullTabs") ?? {}, layoutSidebar(layout()));
    if (!transition) return;
    setShared("annotator.sidebarFocus", transition.state);
    for (const action of transition.actions) run(action.name, action.args);
  }

  markGithubRead(thread, cx) {
    if (this.busy || this.snapshotError || !this.data) return;
    const context = this.context;
    this.busy = true;
    cx.spawn(async cx => {
      try {
        const data = await this.request({ op: "github_read", scope: this.data.snapshot.scope,
          thread: { id: thread.id, fingerprint: thread.fingerprint }, read: !thread.read });
        if (contextKey() === context) this.github = data.github;
      } catch (e) { if (contextKey() === context) this.error = String(e); }
      finally { this.busy = false; cx.notify(); }
    });
  }

  githubCard(thread, t) {
    const queued = this.threadQueue.has(thread.id);
    const enabled = !thread.resolved && !thread.delivery && !thread.incomplete && !this.github?.error;
    return v_flex().id(`thread-${thread.id}`).m(8).p(10).gap(7).rounded(8).bg(t.surface).border_1().border_color(t.border)
      .child(div().text_size(10.5).text_color(thread.read ? t.dim : t.accent).child(thread.read ? "Read" : "● Unread"))
      .child(div().font_semibold().text_size(12).child(`${thread.path}:${thread.start_line ?? thread.line ?? "?"}${thread.start_line ? `–${thread.line}` : ""} · ${thread.side}`))
      .child(div().text_size(11).text_color(t.muted).child(thread.resolved ? "Resolved" : thread.delivery ? `Delivery: ${thread.delivery}` : thread.incomplete ? "Incomplete thread · cannot send" : `${thread.outdated ? "Outdated location · " : ""}${thread.has_updates ? "New replies · " : ""}Open`))
      .children(thread.comments.map(c => v_flex().gap(3)
        .child(div().text_size(11).text_color(t.accent).child(`@${c.author}`))
        .child(div().text_size(12).child(c.body))))
      .child(h_flex().gap(5)
        .child(this.button(`read-thread-${thread.id}`, thread.read ? "Mark unread" : "Mark read", cx => this.markGithubRead(thread, cx)))
        .child(this.button(`queue-${thread.id}`, queued ? "Remove from review" : "Add to review", cx => {
          if (queued) this.threadQueue.delete(thread.id);
          else this.threadQueue.set(thread.id, { id: thread.id, fingerprint: thread.fingerprint });
          cx.notify();
        }, enabled, "secondary"))
        .child(this.button(`fix-${thread.id}`, "Fix thread", cx => this.sendToSession(cx, thread), enabled && !this.editor.value().trim() && this.currentSession()?.status === "idle", "primary")));
  }

  button(id, text, action, enabled = true, variant = "quiet") {
    const t = theme();
    const active = enabled && !this.busy && !(this.snapshotError && needsFreshSnapshot(id));
    return h_flex().id(id).h(29).px(10).rounded(6).text_size(12).items_center().justify_center().flex_none()
      .text_color(active ? t.foreground : t.dim)
      .when(variant === "primary", el => el.bg(active ? t.accent : t.raised)
        .text_color(active ? t.accent_foreground : t.dim).font_semibold())
      .when(variant === "secondary", el => el.bg(t.surface).border_1().border_color(t.border))
      .when(active, el => el.cursor_pointer().hover(s => variant === "primary" ? s.opacity(0.85) : s.bg(t.hover))
        .on_click((_e, cx) => action(cx))).child(text);
  }

  badge(text, color, background = null) {
    return h_flex().px(6).h(19).rounded(4).items_center().text_size(10.5).text_color(color)
      .when(Boolean(background), el => el.bg(background)).child(text);
  }

  selectFile(path, cx) {
    if (this.busy) return;
    if (this.editor.value().trim()) { this.notice = "Save or cancel your draft before selecting another file."; cx.notify(); return; }
    this.open = path; this.draft.reset(); this.activeComment = null; this.notice = null; this.view = "diff"; this.refresh(cx); cx.notify();
  }

  selectLine(row, event, cx) {
    if (this.busy) return;
    this.draft.anchor(selectionFor(this.selection, row, event?.modifiers?.shift), this.data?.snapshot.revision);
    this.activeComment = null; this.composerOpen = true;
    this.exportText = null; this.notice = null; this.editor.focus(); cx.notify();
  }

  commentCard(comment, t, showFile = false) {
    const status = commentStatus(comment);
    const color = t[status.tone];
    const path = pathParts(comment.path);
    const hasDiff = this.data?.snapshot.files.some(f => f.path === comment.path);
    return v_flex().id(`note-${comment.id}`).mx(8).my(6).p(10).gap(7).rounded(8)
      .bg(t.surface).border_1().border_color(t.border).border_l(3).border_color(color)
      .when(showFile, el => el.child(h_flex().items_center().gap(4)
        .child(div().flex_1().min_w(0).truncate().text_size(12).font_semibold().child(path.name))
        .child(this.button(`open-note-${comment.id}`, hasDiff ? "View code" : "No diff", cx => {
          if (!this.busy && !this.editor.value().trim()) this.contextLines = Math.max(this.contextLines, comment.context ?? 3);
          this.selectFile(comment.path, cx);
        }, Boolean(hasDiff)))))
      .when(showFile && Boolean(path.folder), el => el.child(div().text_size(10.5).text_color(t.dim).child(path.folder)))
      .child(h_flex().items_center().gap(6)
        .child(this.badge("YOU", t.muted, t.raised))
        .child(this.badge(comment.read ? "Read" : "Unread", comment.read ? t.dim : t.accent))
        .child(div().flex_1().text_size(11).text_color(t.muted).child(commentLocation(comment)))
        .child(this.badge(status.label, color, color + "18")))
      .child(div().text_size(13).text_color(t.foreground).child(comment.body))
      .when(comment.stale && !comment.resolved && !comment.delivery, el => el.child(div().text_size(11).text_color(t.ansi3)
        .child("Code changed. This comment will not be sent. Add a new comment on the current lines.")))
      .when(comment.delivery === "uncertain", el => el.child(div().text_size(11).text_color(t.ansi3)
        .child("Check the chat before sending again; it may already have received this comment.")))
      .child(h_flex().gap(4).justify_end()
        .child(this.button(`read-${comment.id}`, comment.read ? "Mark unread" : "Mark read", cx => this.act("read", { id: comment.id, read: !comment.read }, cx)))
        .when(Boolean(comment.delivery) && !comment.resolved, el => el.child(this.button(`requeue-${comment.id}`, "Send again…", cx => {
          this.requeue = this.requeue === comment.id ? null : comment.id; cx.notify();
        })))
        .child(this.button(`resolve-${comment.id}`, comment.resolved ? "Reopen" : "✓ Resolve", cx => this.act("resolve", { id: comment.id, resolved: !comment.resolved }, cx), true, "secondary")))
      .when(this.requeue === comment.id, el => el.child(v_flex().gap(4)
        .child(div().text_size(11).text_color(t.ansi3).child("Requeue only after checking the chat. This can duplicate feedback."))
        .child(this.button(`confirm-requeue-${comment.id}`, "Confirm requeue", cx => {
          this.requeue = null; this.act("resolve", { id: comment.id, resolved: false }, cx);
        }, true, "secondary"))));
  }

  fileRow(file, count, t) {
    const path = pathParts(file.path), stats = fileStats(file);
    const active = file.path === this.open;
    const code = file.status.includes("D") ? "D" : file.status.includes("A") || file.status === "??" ? "+" : "M";
    const color = code === "D" ? t.ansi1 : code === "+" ? t.ansi2 : t.ansi3;
    return h_flex().id(`file-${file.path}`).h(42).flex_none().px(10).py(5).gap(8).items_center().cursor_pointer()
      .border_l(3).border_color(active ? t.accent : t.chrome)
      .bg(active ? t.selected : t.chrome).hover(s => s.bg(t.hover))
      .child(h_flex().size(23).rounded(5).bg(color + "18").text_color(color).text_size(11).font_semibold().items_center().justify_center().flex_none().child(code))
      .child(v_flex().flex_1().min_w(0).gap(1)
        .child(div().truncate().text_size(12.5).font_semibold().child(path.name))
        .when(Boolean(path.folder), el => el.child(div().truncate().text_size(10.5).text_color(t.dim).child(path.folder))))
      .child(v_flex().gap(2).items_end().flex_none()
        .child(h_flex().gap(5)
          .when(stats.added > 0, el => el.child(div().text_size(10.5).text_color(t.ansi2).child(`+${stats.added}`)))
          .when(stats.removed > 0, el => el.child(div().text_size(10.5).text_color(t.ansi1).child(`−${stats.removed}`))))
        .when(count > 0, el => el.child(this.badge(`${count} comment${count === 1 ? "" : "s"}`, t.accent, t.accent + "18"))));
  }

  codeRows(file, comments, t) {
    const inline = comments.filter(c => c.path === file.path && !c.stale && c.position);
    return file.rows.flatMap((row, i) => {
      const active = selected(row, this.selection);
      const selectable = !file.truncated && (row.old != null || row.new != null);
      const code = h_flex().id(`diff-${i}`).w_full().min_h(25).py(3).flex_none().gap(5).pr(8).items_start()
        .font_family(settings().font ?? "JetBrains Mono").text_size(12)
        .border_l(3).border_color(active ? t.accent : t.background)
        .when(row.kind === "hunk", el => el.bg(t.raised).text_color(t.dim))
        .when(row.kind === "add", el => el.bg(t.ansi2 + "16"))
        .when(row.kind === "delete", el => el.bg(t.ansi1 + "16"))
        .when(active, el => el.bg(t.selected))
        .when(selectable, el => el.cursor_pointer().hover(s => s.bg(t.hover))
          .on_click((event, cx) => this.selectLine(row, event, cx)))
        .child(div().w(29).flex_none().text_right().text_color(active ? t.accent : t.dim).child(row.old == null ? "" : String(row.old)))
        .child(div().w(29).flex_none().text_right().text_color(active ? t.accent : t.dim).child(row.new == null ? "" : String(row.new)))
        .child(div().w(10).flex_none().text_color(row.kind === "add" ? t.ansi2 : row.kind === "delete" ? t.ansi1 : t.dim)
          .child(row.kind === "add" ? "+" : row.kind === "delete" ? "−" : ""))
        .child(this.codeText(row, null, t, this.syntaxCache?.tokens));
      return [code, ...inline.filter(c => inlineAt(c, row)).map(c => this.commentCard(c, t))];
    });
  }

  render(cx) {
    this.syncSidebar();
    if (contextKey() !== this.context && !this.busy) this.refresh(cx);
    if (contextKey() === this.context && this.machineUnavailable(location().scope) && !this.snapshotError) {
      this.snapshotFailed(`Machine ${location().scope.machine} is unavailable`, location().scope);
    }
    const t = theme();
    const current = contextKey() === this.context;
    if (!current) return div().p(12).text_color(t.dim).child("Finishing the previous worktree's request…");
    if (this.full) return renderFull(this, cx);
    const data = current ? this.data : null;
    const files = data?.snapshot.files ?? [];
    const comments = data?.comments ?? [];
    const commentCounts = new Map();
    for (const comment of comments) commentCounts.set(comment.path, (commentCounts.get(comment.path) ?? 0) + 1);
    const file = files.find(f => f.path === this.open);
    const fileComments = file ? comments.filter(c => c.path === this.open) : [];
    const other = fileComments.filter(c => c.stale || !c.position);
    const missing = comments.filter(c => !files.some(f => f.path === c.path));
    const pending = pendingComments(comments);
    const target = this.currentSession();
    const selection = this.selection;
    const canSave = Boolean(file && this.editor.value().trim() && !this.needsSelection);
    const summary = reviewSummary(comments);
    const hasDraft = Boolean(this.editor.value().trim());
    const source = file ? pathParts(file.path) : null;
    const stats = file ? fileStats(file) : null;
    const chat = target?.program?.name ?? (target?.app === "opencode" ? "OpenCode" : target?.app) ?? "Agent";
    const batchSize = pending.length + this.threadQueue.size;
    const sendLabel = batchSize ? `Send review · ${batchSize}` : "Send review";
    return v_flex().id("annotator-root").size_full().min_h(0).bg(t.background).text_color(t.foreground).text_size(13).track_focus(this.handle)
      .on_key_down((event, cx) => this.handleKey(event, cx))
      .child(v_flex().flex_none().bg(t.chrome).px(10).py(6)
        .child(h_flex().items_center().gap(7)
          .child(div().font_semibold().text_size(14).child("Changes"))
          .child(div().flex_1().min_w(0).truncate().text_size(11).text_color(t.dim)
            .child(`${data?.snapshot.scope.machine ? data.snapshot.scope.machine + " · " : ""}${data?.snapshot.branch ?? ""}`))
          .child(this.button("open-full-review", "↗", cx => this.openFull(cx), Boolean(data)).tooltip("Open full review tab"))
          .child(this.button("review-refresh", this.busy ? "…" : "↻", cx => { this.refresh(cx, true); this.refreshGithub(cx, true); }).tooltip("Refresh diff and GitHub"))))
      .when(Boolean(this.error), el => el.child(this.errorCard(t)))
      .when(Boolean(this.notice), el => el.child(div().mx(8).my(4).p(8).rounded(6).bg(t.raised).text_size(11.5).text_color(t.muted).child(this.notice)))
      .child(h_flex().h(33).flex_none().px(6).gap(4).bg(t.chrome).border_b(1).border_color(t.border)
        .child(this.button("view-diff", "Code", cx => { this.view = "diff"; cx.notify(); }, true, this.view === "diff" ? "secondary" : "quiet"))
         .child(this.button("view-comments", `Comments · ${summary.total}`, cx => { this.view = "comments"; cx.notify(); }, true, this.view === "comments" ? "secondary" : "quiet"))
         .child(this.button("view-github", `GitHub · ${this.github?.threads?.length ?? 0}`, cx => { this.view = "github"; this.refreshGithub(cx); cx.notify(); }, true, this.view === "github" ? "secondary" : "quiet"))
         .when(summary.outdated > 0, el => el.child(this.badge(`${summary.outdated} outdated`, t.ansi3, t.ansi3 + "18"))))
      .child(h_flex().id("files-scope-row").h(31).flex_none().px(5).gap(3).items_center().bg(t.chrome)
        .when(this.view === "diff", el => el.child(this.button("toggle-files", `${this.filesOpen ? "▾" : "▸"} Files · ${data ? files.length : "—"}`, cx => { this.filesOpen = !this.filesOpen; cx.notify(); })))
        .when(this.view !== "diff", el => el.child(this.button("filter-unread", this.onlyUnread ? "● Unread only" : "All comments", cx => { this.onlyUnread = !this.onlyUnread; cx.notify(); })))
        .child(div().flex_1().min_w(0))
        .child(this.button("scope-menu", `${this.mode === "branch" ? "Branch" : "Uncommitted"} ▾`, cx => { this.scopeMenuOpen = !this.scopeMenuOpen; this.baseOpen = false; cx.notify(); }, true, "secondary"))
        .when(this.mode === "branch", el => el.child(this.button("choose-base", `${this.baseLabel()} ▾`, cx => { this.baseOpen = !this.baseOpen; this.scopeMenuOpen = false; cx.notify(); })
          .flex_shrink(1).min_w(0).max_w(145).truncate().tooltip(this.base ?? "Choose base branch"))))
      .when(this.scopeMenuOpen, el => el.child(h_flex().id("scope-options").px(8).gap(4).bg(t.chrome)
        .child(this.button("scope-uncommitted", "Uncommitted", cx => this.changeComparison("uncommitted", null, cx), true, this.mode === "uncommitted" ? "secondary" : "quiet"))
        .child(this.button("scope-branch", "Branch", cx => this.changeComparison("branch", this.base, cx), true, this.mode === "branch" ? "secondary" : "quiet"))))
      .when(this.baseOpen, el => el.child(v_flex().id("review-bases").h(140).overflow_y_scrollbar()
        .child(this.button("base-auto", "Auto · prefer local develop", cx => this.changeComparison("branch", null, cx, false)))
        .children(this.bases.map(base => this.button(`base-${base}`, base, cx => this.changeComparison("branch", base, cx, true))))))
      .child(v_resizable("annotator-sections").flex_1().min_h(0)
        .on_resize((sizes, cx) => { this.sizes = sizes; setShared("annotator.sections", sizes); cx.notify(); })
        .child(resizable_panel().size(this.sizes[0]).size_range(50).visible(this.view === "diff" && this.filesOpen && files.length > 0)
          .child(this.view === "diff" && this.filesOpen ? v_flex().id("review-files").relative().size_full().min_h(0)
            .child(v_virtual_list("review-files-list", files.length, 42,
              index => files[index].path,
              range => files.slice(range.start, range.end).map(f => this.fileRow(f, commentCounts.get(f.path) ?? 0, t)))
              .size_full().on_item_click((path, cx) => this.selectFile(path, cx)))
            .child(Scrollbar.vertical("review-files-list").absolute().inset_0()) : div()))
        .child(resizable_panel().size(this.sizes[1]).size_range(90).child(v_flex().size_full().min_h(0)
      .when(this.view === "diff", el => el
        .when(Boolean(file), el => el.child(v_flex().flex_none().px(12).py(7).gap(3).border_t(1).border_b(1).border_color(t.border)
          .child(h_flex().gap(6).items_center().child(div().flex_1().min_w(0).truncate().font_semibold().text_size(12.5).child(source?.name))
            .when(stats?.added > 0, el => el.child(this.badge(`+${stats.added}`, t.ansi2)))
            .when(stats?.removed > 0, el => el.child(this.badge(`−${stats.removed}`, t.ansi1))))
          .when(Boolean(source?.folder), el => el.child(div().text_size(10.5).text_color(t.dim).truncate().child(source.folder)))
          .child(div().text_size(10.5).text_color(t.dim).child("Click a line to comment · Shift-click for a range"))
          .child(h_flex().gap(4)
            .child(this.button("context-more", "+20 context", cx => this.expandContext(Math.min(1000000, this.contextLines + 20), cx), this.contextLines < 1000000))
            .child(this.button("context-all", "Full context", cx => this.expandContext(1000000, cx), this.contextLines < 1000000))
            .child(this.button("context-reset", "Collapse", cx => this.expandContext(3, cx), this.contextLines > 3)))))
        .child(v_flex().id("review-diff").flex_1().min_h(0).overflow_y_scrollbar().children([
          ...(file?.note ? [div().m(8).p(8).rounded(5).bg(t.raised).text_size(11).text_color(t.muted).child(file.note)] : []),
          ...(file ? this.codeRows(file, comments, t) : [v_flex().p(18).gap(7)
            .child(div().font_semibold().child(this.busy ? "Loading changes…" : this.snapshotError?.title ?? (data ? "No changes to review" : "Select a Git checkout")))
            .child(div().text_size(12).text_color(t.muted).child(this.snapshotError ? "Changes could not be loaded. No files or saved comments were removed." : data ? "Saved feedback is kept in Comments, even when the code is committed." : "Open a project space to review its uncommitted changes."))]),
          ...other.map(c => this.commentCard(c, t)),
          ...missing.map(c => this.commentCard(c, t, true)),
        ])))
      .when(this.view === "comments", el => el.child(v_flex().id("review-comments").flex_1().min_h(0).overflow_y_scrollbar().py(4)
        .children(unreadItems(comments, this.onlyUnread).map(c => this.commentCard(c, t, true)))
        .when(!comments.length, el => el.child(v_flex().p(18).gap(8)
          .child(div().font_semibold().child("Your review starts with a comment"))
          .child(div().text_size(12).text_color(t.muted).child("Choose a file in Code, select a line, and write what should change. Save several comments, then send them together."))))))
      .when(this.view === "github", el => el.child(v_flex().id("review-github").flex_1().min_h(0).overflow_y_scrollbar()
        .child(div().p(10).text_size(11).text_color(t.muted).child(this.githubBusy ? "Checking GitHub…" : this.github?.pull_request ? `${this.github.pull_request.url} · polls every 5 minutes · read-only` : "No open PR found for this branch. Refresh checks GitHub now."))
        .when(Boolean(this.github?.error), el => el.child(div().p(10).text_color(t.ansi3).child(this.github.error)))
        .when(Boolean(this.github?.limited), el => el.child(div().p(10).text_color(t.ansi3).child("GitHub thread limit reached; more threads exist on the PR.")))
        .children(unreadItems(this.github?.threads ?? [], this.onlyUnread).map(thread => this.githubCard(thread, t)))))))
        .child(resizable_panel().size(this.sizes[2]).size_range(110).child(v_flex().size_full().min_h(0).overflow_y_scrollbar().child(v_flex().mx(8).mt(6).mb(8).p(9).gap(6).rounded(8).bg(t.surface).border_1().border_color(selection ? t.accent : t.border)
        .child(h_flex().items_center().gap(5)
          .child(div().flex_1().min_w(0).truncate().font_semibold().text_size(11.5).text_color(selection ? t.accent : t.foreground)
            .child(selection ? `Comment on ${selection.side === "old" ? "old " : ""}L${selection.start}${selection.end === selection.start ? "" : `–${selection.end}`}` : "Comment on this file"))
          .when(Boolean(selection) || this.needsSelection, el => el.child(this.button("file-note", "Whole file", cx => this.fileNote(cx), Boolean(file)))))
        .when(!selection, el => el.child(div().text_size(10.5).text_color(t.dim).child(file ? source.name : "Select a changed file first")))
        .child(div().rounded(5).bg(t.background).border_1().border_color(t.border).p(6)
          .child(Textarea.new(this.editor).h(65).w_full().text_size(13)))
        .child(h_flex().gap(5).items_center()
          .child(div().flex_1().text_size(10.5).text_color(t.dim).child(hasDraft ? "⌘/Ctrl+Enter saves · not sent" : "Saved comments join your review"))
          .when(hasDraft || Boolean(selection), el => el.child(this.button("cancel-note", "Cancel", cx => {
            this.cancelDraft(cx);
          })))
          .child(this.button("save-note", "+ Save comment", cx => this.saveComment(cx), canSave, "primary").tooltip("Save comment · ⌘Enter / Ctrl+Enter")))))))
      .when(Boolean(this.exportText), el => el.child(v_flex().mx(8).mb(8).p(8).gap(4).rounded(7).bg(t.surface).border_1().border_color(t.border).flex_none()
        .child(div().text_size(11).text_color(t.muted).child("Review text · select and copy"))
        .child(Textarea.new(this.output).h(95).w_full()).child(this.button("close-export", "Close", cx => { this.exportText = null; cx.notify(); }))))
      .child(v_flex().flex_none().p(10).gap(6).bg(t.chrome).border_t(1).border_color(t.border)
        .child(h_flex().gap(7).items_center()
          .child(h_flex().size(27).rounded(7).bg(t.accent + "18").text_color(t.accent).items_center().justify_center().font_semibold().child("↗"))
          .child(v_flex().flex_1().min_w(0).gap(1)
            .child(div().id("review-session").font_semibold().text_size(12).truncate().child(target ? `To ${chat} · ${target.address}` : "No chat selected"))
            .child(div().text_size(10.5).text_color(t.dim).child(`${pending.length} comments · ${this.threadQueue.size} GitHub threads ready`)))
          .when(Boolean(target), el => el.child(this.badge(target.status === "idle" ? "Ready" : target.status === "blocked" ? "Needs you" : "Working", target.status === "idle" ? t.ansi2 : t.ansi3)))
          .child(this.button("export-review", "Export…", cx => this.act("export", { threads: [...this.threadQueue.values()] }, cx), batchSize > 0)))
        .child(this.button("send-review", sendLabel, cx => this.sendToSession(cx), batchSize > 0 && target?.status === "idle" && !hasDraft, "primary").w_full().h(33))
        .child(div().text_size(10.5).text_color(t.muted).child(this.snapshotError ? "Refresh must succeed before saving or sending feedback." : this.threadQueue.size && !hasDraft && target?.status === "idle" ? "One click submits comments and selected GitHub threads to this session." : sendHint(comments, target, hasDraft))));
  }
}
