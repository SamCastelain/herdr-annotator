# Herdr Annotator

A code-review mod for **[Herdr App](https://herdr.dev)** (`herdr-app`). Review
changes, leave comments, and send feedback to your agent session.

## Features

- Changes sidebar and a full review tab with unified or split diffs.
- Branch and uncommitted changes, local or over SSH. Auto base prefers local `develop`.
- File and line comments, read/unread markers, resolve/reopen, and export.
- Syntax coloring, expandable context, resizable panes, and large file lists.
- Read-only GitHub review threads with refresh, polling, and **Fix thread**.
- Saved view settings and **⌘Enter / Ctrl+Enter** to save a comment.

## Install

Requires macOS or Linux, Herdr App, herdr-core (`hc`), Git, Rust, and Python 3.
GitHub threads also need an authenticated GitHub CLI (`gh`) on the app machine.

```sh
git clone https://github.com/jakub-tucek/herdr-annotator.git
cd herdr-annotator
python3 scripts/install.py
```

The installer builds the Rust helper and installs the native JavaScript mod.
Remote machines need no extra installation.

To use it in the existing Changes panel, clone a layout and install into that copy:

```sh
herdr-app do layout.clone '{"name":"main","as":"my-review"}'
python3 scripts/install.py --layout my-review
```

For an existing personal layout, use its name instead. The installer backs up
changes and refuses to overwrite locally edited mod files.

## Use

Open **Changes**, select a file, and click a line to comment. Shift-click selects
a range. Save comments with the button or **⌘Enter / Ctrl+Enter**.

Click **↗** for full review. It hides the duplicate sidebar and keeps the source
checkout and agent binding. Click **Send review** to submit saved feedback to the
idle agent, then return to its session.

Comments stay on your app machine. Git and GitHub review reads do not modify the
repository or post replies. **Send review submits with Enter**; check the chat
before retrying uncertain delivery. Diff preview limits still apply.

## Development

```sh
cargo test --locked
npm test # Node.js 22+
```

See the [user guide](docs/guide.md), [JSON interface](docs/interface.md), and
[mod recipe](RECIPE.md) for details.

[MIT license](LICENSE).
