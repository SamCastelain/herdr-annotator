# Herdr Annotator

Review your agent's code without leaving Herdr. Compare branch, last commit, or uncommitted
changes in a native sidebar or full review tab, leave comments on files or lines,
and send your review to the agent working in that checkout. Read GitHub PR review
threads, reply directly, or queue them for the agent to address. Works with local
and SSH worktrees.

![Herdr Annotator](preview.png)

## What you see

One compact Changes panel: title/branch, full-tab and refresh icons; view tabs;
Files row with Uncommitted/Branch dropdown and local base refs;
collapsible file list; file title and range-selection hint;
scrollable wrapped unified diff with old/new gutters; bordered inline note cards;
boxed multiline composer; fixed session recipient and full-width send action.
File rows separate the filename and directory, plus green/red line counts and
comment-count badges. Use theme colors and actual sidebar width, not a wide tab.
Native draggable dividers separate files, code/comments, and composer; sizes persist.
Context buttons expand unchanged lines by 20, request full context, or collapse.
The full-tab icon opens a pinned standalone review: collapsible file navigator
and wide virtualized unified/split diff with inline comments. A third comments
column is optional, off by default. The composer opens on demand.
Full reviews default to Branch and remember comparison mode, diff style, pane
visibility, unread filter, context size and widths. Base refs stay checkout-specific.
Syntax/Plain is a saved toggle. A bounded lexer colors source tokens with theme
colors without changing source text or the diff's add/delete/selection backgrounds.
Inline note cards use 8px padding, 4px gaps and 12px body text. No instructional
footer repeats beneath each note. Click an open unsent card to edit; other cards
open read/resolve/requeue actions.
Keep a fixed native notes slot before the file and after each source row, even
when empty. gpui-shell 0.7.1 splices the entire measured list when item count
changes, resetting the reading position. Adding/removing/filtering comments
must change slot contents, not list identity, item count or source-row keys.
Clicking a slot with multiple notes opens their individual action cards.

Added/deleted source has green/red tints; selection has an accent gutter and
background. Ready-to-send note cards use accent borders and badges; outdated
cards use amber with an explicit exclusion explanation; sent/resolved notes have
their own status. Comments lists every file's feedback, including outdated and
no-longer-changed files. Saving and sending are separate buttons with distinct
placement. Disabled Send explains whether a draft, missing chat, busy agent or
outdated comments block it. No terminal UI or popup picker.

## How it behaves

Default to the active space's terminal checkout, with optional pinned scope props.
Folders outside Git show a neutral Not a Git checkout state with Refresh, not a
red error. No selected folder asks for a checkout. Preserve drafts and block stale
review actions; connection, permission and corrupt-repository failures stay errors.
File notes use no range. Clicking old-only deletions selects old lines; other
source rows select new lines. Shift-click extends on the same side. Saved notes
support resolve/reopen; unsaved drafts stay in memory per context. Refresh every
15 seconds except during a draft. A changed revision requires reselection.
List all changed files, loading only selected/annotated diffs. Branch scope includes
committed and uncommitted work since merge-base; never fetch refs automatically.
Unchanged lines accept annotations; collapsing the preview preserves their anchors.

GitHub threads poll every five minutes; Refresh fetches immediately. Show replies,
outdated/resolved states, new replies and delivery receipts. Fix thread sends only
that thread; Add to review queues it with local notes. Reply opens a separate
composer; only Post to GitHub writes a reply, using the signed-in gh account.
Keep reply drafts per context/thread. Persist uncertainty before posting; never
retry automatically. Explicit retry requires the person to check GitHub first.
Missing gh or sign-in shows installation/auth guidance, without blocking local reviews.
Read/unread markers persist locally. Unread-only filters comment lists; new GitHub
replies become unread. Reading changes neither resolution nor delivery eligibility.
Hide resolved and confirmed-sent local notes and GitHub threads by default,
including inline notes and file badges. Show sent/resolved / Hide sent/resolved
reveals or hides saved history; failed/uncertain sends remain visible. Sending
does not resolve threads on GitHub or claim the feedback is fixed;
remember this choice across the sidebar and full review. Local notes can reopen.
Full tabs pin the source checkout/comparison/chat. Losing that chat disables Send.
Opening full review hides the existing Changes sidebar; returning to the source
tab restores it. Respect manual sidebar toggles and do not replace a Files panel.

Bind to the current tab's focused agent session automatically. Keep that binding
when the composer takes focus; never guess between multiple chats or across tabs.
Each Send click resolves the current chat again and submits with Enter immediately,
without an extra confirmation. Disable repeat clicks while delivery is running.
Working/blocked/unreported terminals cannot receive feedback. Persist uncertain
delivery before the core call so a crash/reply loss never silently resends.
Requeue requires another explicit decision. Tests never send to a real agent.
After confirmed delivery, focus the exact recipient pane. Never focus another
chat if it disappeared, and never change focus on failed or uncertain delivery.
Cmd+Enter / Ctrl+Enter in the composer saves one comment, never sends the batch.
Plain Enter stays multiline; the save action obeys stale/offline/busy guards.
Local Edit/Delete require the expected saved body and current revision. Preserve
the anchor and delivery history; only open, unsent notes are editable/deletable.
Delete asks for confirmation. Concurrent edits must fail rather than overwrite.

Last commit compares HEAD's first parent to the working tree, including staged,
unstaged and untracked work. Root commits use the empty tree; unborn HEAD has no
Last commit scope. Keep annotations isolated by comparison mode.

Right-click a source line or hunk to ask a question, or ask about the selection
or open diff from a button/command. Questions have their own composer and do not
send saved review notes. Validate the snapshot, complete preview, diff side and
source bounds. Send only to the idle source chat. Persist content-keyed private
uncertain/confirmed receipts before/after delivery; explicit checked retries
must keep the original text and target. Cap messages at 64 KiB.

Commands annotator.submit/ask/scope/refresh/full use nextCommand and existing
guards, with no global key changes. One-shot shared annotator.open accepts
{id,at,path,on}; refresh and select only a matching checkout/machine, keep drafts
and scope, expire after 30 seconds, and never clear a newer request.

Check published stable GitHub releases daily, using public API metadata only;
show version, release notes, Update and Later. No code fetch/build without an
explicit click. annotator.updates opens manual status/checks. Bootstrap existing
installs with one manual installer run. Keep package/mod versions aligned.
Use an application-session view registry: hidden drafts and in-flight operations
block Update, without heartbeat expiry. Pause editors globally during install.
Probe the durable worker status on app restart before accepting new drafts.
A detached Python worker holds a file lock through install, verifies receipts,
refuses dirty/diverged source and locally modified installs, fetches the fixed
official HTTPS upstream/tag, verifies its pinned SHA and version, fast-forwards
only, then runs the installer with --expected-commit. Never reset/stash/discard.
Check source cleanliness before and after the build; back up replaced files.
Keep layouts intact during automatic installation and update private tabs from
their receipt. Persist version/commit/binary/updater hashes. Builds need the
existing Rust/Python/Git toolchain; no global keys or background daemon install.

Auto branch base prefers local develop before origin/HEAD. Named base selections
are pinned; Auto clears the pin. Reset legacy origin/develop defaults to Auto,
preserve custom saved refs, and keep notes isolated by resolved comparison/base.

## Built from

- `mods/annotator/main.js`: native GPUI view, TextareaState composer, themed rows,
  context-scoped drafts, current-tab session binding, per-call async contexts;
  helpers in `review.js`.
- Rust CLI/library: `Engine::handle(Request)` with snapshot/add/resolve/export/send/github.
- `src/question.rs`: bounded question context and durable delivery receipts;
  Edit/Delete and Last commit use the same prepared snapshot and private stores.
- Read markers: `read` for local comments and `github_read` for cached thread versions.
- `mods/annotator/full.js`: full-tab layout, deferred variable-height diff rows,
  split-side selection and paged comment cards. `list` comes from gpui-kit.
- `scripts/tabs_integration.py` and `mods/annotator/launcher.js`: installer-generated
  private `annotator-tabs` mod adds a direct full-review button beside Changes.
  `--layout` updates personal layouts and receipt-checks the private copy; shipped
  tabs remain untouched. Source checkout/chat are captured before creating the tab.
- `mods/annotator/syntax.js`: bounded pure lexical tokens; independent old/new
  multiline state, reset across hidden context gaps. No HTML or code evaluation.
- Read-only Git subprocesses; remote Git argv through `hc exec`. Local hashed
  JSON stores with exclusive locks, atomic mode-0600 writes and corruption errors.
- Exact selected-text/context anchoring; refuse ambiguity, branch mismatches,
  stale revisions, noncontiguous/out-of-preview selections.
- Mod methods: `spaces`, `panes`, `focused`, `agents`, `exec`, `home`, `props`,
  `theme`, `settings`, `shared`, `setShared`; native `v_resizable`/`resizable_panel`.
  GitHub API queries run through local authenticated `gh`, only on github.com.

## Watch out

Herdr App mods are JavaScript, not Rust dynamic libraries. Rust is a local helper;
remote cores do not load this mod. `hc send` includes Enter. The mod's `exec`
has a 30-second limit. Native checks render only the initial state; also inspect
the live panel. Anchor old/new sides separately and keep literal Git pathspecs.
The helper shares a 25-second subprocess budget across each request, including
GitHub pagination and Send. Capture drains both pipes with bounded retention;
inherited writers cannot keep it waiting indefinitely. Local process-group
cleanup does not confirm remote cancellation. Unconfirmed Send stays uncertain.
Honor documented preview limits, preserve stale notes and never overwrite
corrupt stores. Do not edit shipped mods/layouts or stage/commit review targets.

## Done when

Rust/JS tests pass; cargo fmt/clippy pass; Herdr check passes. A real review tab
shows changed files and saved inline notes. Notes survive a helper restart, move
only to unique exact context, and become outdated for modified/ambiguous anchors.
Fake-core tests cover remote reads, confirmed delivery, failure uncertainty,
duplicate prevention and busy/blocked refusal. Git index bytes stay unchanged.
Large-list tests load a file past index 100; context notes survive collapsing.
Fake-GitHub tests cover paging, caching, reply changes, explicit posting, missing
gh, and duplicate prevention after a lost confirmation. Tests never post to real PRs.
