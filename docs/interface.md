# JSON interface, version 1

One process handles one request. All paths/strings are JSON, not shell text.
The helper always runs on the app machine; `scope.machine` selects an existing
core connection. The supplied `scope.repo` may be inside a checkout; Git resolves
its root. Send the canonical scope returned by `snapshot` for later operations.

```json
{"op":"snapshot","scope":{"repo":"/code/project","machine":null}}
```

Result data: `{snapshot, comments}`. Snapshot contains canonical `scope`,
`branch`, `head`, opaque `revision`, `files`, `limited`, `bases`, `merge_base`, `view`.
Files contain `path`, Git `status`, `rows`, `note`, `truncated`, `loaded`. Rows contain `kind`
(`hunk`, `context`, `add`, `delete`, `meta`), `old`/`new` source numbers or null,
and exact source `text` with its diff prefix removed. Comment views contain
persisted fields plus `stale` and current `position` (null for file notes or stale
notes; use `stale` to distinguish).

`scope.comparison` defaults to `{"mode":"uncommitted","base":null}`. Set
`{"mode":"branch","base":"origin/main"}` to compare merge-base with the
current worktree, including untracked files. Omitting a branch base selects a
locally available default; no fetch occurs. Branch/base scopes have separate stores.
Auto prefers local `develop` before `origin/HEAD` or remote-tracking defaults.
An explicit `comparison.base` is always respected, including `origin/develop`.

Every non-GitHub operation accepts `view`, default
`{"file":null,"context":3}`. All file metadata is returned, while only the
selected/default-first file and files with stored notes load diffs. `loaded:false`
means rows have not been requested, not that the file is empty. Select a filename
in `view.file` to load it. `view.context` is 0–1000000 unchanged lines per hunk.
Preview byte/row limits still apply. Use the returned `snapshot.view` with the
returned revision on add/export/send; changing the view changes the revision.
Notes retain their creation context for anchor validation after collapsing.

```json
{
  "op":"add",
  "scope":{"repo":"/code/project","machine":null},
  "revision":"<snapshot revision>",
  "path":"src/main.rs",
  "selection":{"side":"new","start":12,"end":15},
  "body":"Handle a missing value here."
}
```

Use `selection: null` for file notes. Range endpoints are inclusive. The engine
captures the anchor from its fresh Git snapshot, not caller-supplied text. Saving
rejects stale revisions, hidden ranges, missing files, truncated diff ranges,
empty/oversized notes and unknown fields. Adds return updated `{snapshot,comments}`.

```json
{"op":"resolve","scope":{"repo":"/code/project","machine":null},"id":"<comment id>","resolved":true}
```

`resolved: false` reopens and explicitly clears a sent/uncertain delivery marker.
The UI requires an extra confirmation to requeue a delivery-marked open note.
Resolve returns the updated review; it does not need a diff revision because it
changes the note state, not its source anchor.

```json
{"op":"read","scope":{"repo":"/code/project","machine":null},"id":"<comment id>","read":true}
```

`read:false` makes it unread again. Returns the updated review and accepts the
same optional `view` as Resolve. No revision is needed. Read markers default to
false for old stores; they never change resolution, delivery or export eligibility.

```json
{"op":"export","scope":{"repo":"/code/project","machine":null},"revision":"<snapshot revision>"}
```

Returns `{text}` containing current open unsent human feedback. Empty batches and
stale revisions are errors. Export never sends or marks anything delivered.

```json
{"op":"send","scope":{"repo":"/code/project","machine":"server"},"revision":"<snapshot revision>","target":"server/t12"}
```

Send submits with Enter. The caller must obtain an explicit Send action from the
user; the mod sends on that single click, without a second confirmation.
Targets must be terminal IDs on the same machine, reported idle agents, and
currently in that exact checkout. The target is validated via `hc get`, then
submitted with `hc send --if idle` (no force). Pending delivery is persisted
before sending; any unconfirmed reply leaves notes `uncertain`, blocking an
automatic retry. Success marks only that batch `sent` and returns the updated
review. Review completion/resolution is always a separate decision.

## GitHub

```json
{"op":"github","scope":{"repo":"/code/project","machine":null},"force":false,"poll_seconds":300}
```

Returns `{github}` with `pull_request`, `threads`, `checked_at`, `error`, and
`limited`. Uses local authenticated `gh` for github.com only. `force:true` bypasses
the cache; otherwise the interval defaults to 300 seconds and clamps to 30–3600.
Errors preserve the last successful result but prohibit sending its threads.
Thread records include `id`, `fingerprint`, path/line/side, state, comments,
snippet, delivery status and `has_updates`. Threads and replies are paginated.

`export` and `send` accept `threads:[{"id":"…","fingerprint":"…"}]` and
`include_local:true` by default. `include_local:false` sends only the selected
threads. Selected versions are refreshed and checked before delivery. Changed,
resolved, incomplete, sent or uncertain versions are rejected. Receipt persistence
protects against duplicate/uncertain sends. No GitHub mutations are performed.

There is no daemon, HTTP listener, arbitrary shell method, repository write
operation or TUI integration. The helper delegates credentials to local `gh`.

```json
{"op":"github_read","scope":{"repo":"/code/project","machine":null},"thread":{"id":"…","fingerprint":"…"},"read":true}
```

Returns `{github}`. Sets a local marker for the cached thread fingerprint, with
no GitHub API call. Unknown or changed cached versions are rejected. New replies
are unread when refreshed. `read:false` clears the marker. This never changes
resolution, delivery receipts or batch eligibility.
