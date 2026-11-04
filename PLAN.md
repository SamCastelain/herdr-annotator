# Native Windows support plan

## Goal and scope

Run Herdr Annotator in native Windows Herdr App, against local Windows
checkouts and supported SSH checkouts, without requiring WSL or Git Bash.
Preserve existing macOS and Linux behavior.

Herdr's local manual (`~/.config/herdr-app/AGENTS.md`) already documents
Windows support for the host UI, terminals, file watching, and commands.
The main work is in Annotator's helper, installation, update machinery,
and path handling—not a rewrite of the GPUI interface.

This plan is based on source inspection. Windows behavior has not yet been
verified. Confirm the installed Windows Herdr APIs and configuration paths
before committing to platform conventions.

## 1. Establish a native Windows baseline

- [ ] Provision a Windows development/test environment with Herdr App,
      herdr-core (`hc`), Git for Windows, Rust, Python 3, and Node.js 22+.
- [ ] Install and authenticate `gh` for optional GitHub integration tests.
- [ ] Record Windows Herdr's actual app-home location, `home()` and
      `appHome()` values, command discovery, and `exec` behavior.
- [ ] Confirm the supported Rust target/toolchain and build prerequisites.
- [ ] Inventory native executable and npm launcher forms for optional AI
      runners; do not assume `.cmd` files execute like Unix scripts.
- [ ] Run the existing checks to capture compile failures and nonportable
      test assumptions before changing implementation.

## 2. Add a Windows subprocess backend

Primary file: `src/process.rs`.

- [ ] Separate shared request deadlines, output limits, result types, and
      diagnostics from platform-specific process and pipe management.
- [ ] Keep Unix process groups, signals, `fcntl`, and `poll` behind Unix
      configuration gates.
- [ ] Implement Windows child lifetime management, preferably with Job
      Objects, including a safe strategy for assigning spawned processes
      before descendants can escape management.
- [ ] Capture stdout and stderr concurrently with bounded retention and
      fair draining, even when output exceeds retention limits.
- [ ] Ensure inherited pipe handles cannot extend capture indefinitely
      after the direct child exits. Do not introduce indefinitely blocked
      reader threads or cleanup waits.
- [ ] Preserve shared 25-second request budgets, 120-second generation
      budgets, and the bounded post-exit drain period.
- [ ] Preserve diagnostics on timeout, strict UTF-8 stdout handling, lossy
      stderr decoding, truncation flags, and nonzero exit handling.
- [ ] Bound cleanup for errors, timeouts, and unwinding; close owned handles.
- [ ] Avoid visible console windows for background helper commands where
      appropriate, without breaking core or runner invocation.
- [ ] Preserve the rule that local cancellation does not prove remote
      delivery was cancelled or never happened.
- [ ] Scope any new Windows dependencies in `Cargo.toml` appropriately.

## 3. Make executable discovery and invocation portable

Primary files: `src/executables.rs`, `src/git.rs`, `src/github.rs`,
`src/ordering.rs`, and `src/ordering_models.rs`.

- [ ] Replace unconditional Unix permission checks with platform-specific
      executable validation.
- [ ] Resolve the user's home on Windows without requiring `HOME`.
- [ ] Search Windows executable extensions and appropriate user install
      locations while retaining explicit environment overrides.
- [ ] Keep installer-recorded executable paths as the preferred discovery
      source and validate receipt paths before use.
- [ ] Audit direct `git`, `gh`, and `hc` launches for consistent discovery
      from a desktop environment with a restricted PATH.
- [ ] Handle npm `.cmd` launchers explicitly and safely. Prefer invoking
      Node and a resolved script entry point where practical; do not build
      shell command strings containing review content.
- [ ] Verify OpenCode and Pi runner invocation, including PATH passed
      through local `hc exec` and discovery of Pi's Node runtime.
- [ ] Test paths and arguments containing spaces, parentheses,
      apostrophes, Unicode, and shell metacharacters.
- [ ] Retain the existing policy of not sourcing interactive shell files
      merely to discover tools.

## 4. Port installation and runtime configuration

Primary files: `scripts/install.py` and `mods/annotator/main.js`.

- [ ] Define platform-aware helper, receipt, updater, and backup paths.
      Keep existing Unix installations compatible.
- [ ] Respect `HERDR_APP_HOME` and use the verified Windows Herdr app-home
      default rather than guessing it.
- [ ] Select `target/release/herdr-annotator.exe` on Windows and preserve
      the extension in installed and backup binary names.
- [ ] Record the installer's actual Python interpreter (`sys.executable`)
      and use it for subsequent updater launches instead of `python3`.
- [ ] Expose installed helper and updater configuration to the mod, rather
      than relying only on hardcoded Unix paths. Preserve helper overrides
      and propagate configuration into full-review tabs.
- [ ] Use explicit UTF-8 encoding for Python text files and subprocess
      output where the protocol requires UTF-8.
- [ ] Verify binary replacement behavior while helpers are running;
      Windows executable sharing rules may require retries or versioned
      installation paths rather than replacing an in-use executable.
- [ ] Preserve receipts, digest checks, backups, refusal to overwrite
      unrelated or locally edited files, and release-source verification.
- [ ] Verify standalone installation, personal-layout integration,
      private tabs integration, reinstall, and upgrade.
- [ ] Run `herdr-app check` for both the main mod and generated tabs mod.

## 5. Make state storage safe on Windows

Primary files: `src/lib.rs` and `src/store.rs`.

- [ ] Add a Windows default state directory without requiring `HOME`.
      Preserve `HERDR_ANNOTATOR_STATE_DIR` and existing Unix conventions.
- [ ] Keep Unix file modes behind Unix configuration gates and document
      the Windows permissions model used for private review data.
- [ ] Gate directory syncing in `write_atomic` by platform or use a
      supported equivalent. The current unconditional directory open/sync
      must not turn successful Windows writes into reported failures.
- [ ] Verify replacement of existing state files, temporary-file cleanup,
      crash behavior, and errors caused by file sharing restrictions.
- [ ] Verify `fs2` locking across concurrent helper processes on Windows.
- [ ] Test comment persistence, concurrent writes, corrupt-state refusal,
      and durable uncertainty records for agent and GitHub delivery.

## 6. Normalize filesystem paths without changing Git paths

Primary files: `mods/annotator/main.js`, `mods/annotator/review.js`,
`src/git.rs`, and `src/ordering_models.rs`.

- [ ] Distinguish absolute host filesystem paths from Git's slash-separated
      repository-relative paths. Do not blindly rewrite both categories.
- [ ] Update the `annotator.open` API validation to accept Windows drive
      paths and supported UNC paths, as well as Unix absolute paths.
- [ ] Preserve rejection of NULs, traversal, wrong-machine requests, and
      paths outside the reviewed checkout.
- [ ] Use component-aware containment checks rather than raw string
      prefixes; test sibling directories with similar names.
- [ ] Handle separators, drive-letter casing, trailing separators, and
      extended-length canonical path prefixes consistently.
- [ ] Compare canonical checkout identities consistently in source-model
      matching; do not compare raw Git output to a Windows canonical path.
- [ ] Audit exact directory-string matching in source-session lookup.
- [ ] Keep remote path semantics tied to the target machine, not the
      Windows app machine. Test Windows-to-Unix SSH reviews explicitly.
- [ ] Audit display helpers that split paths on `/`; retain Git-relative
      behavior and add host-path handling only where necessary.
- [ ] Test CRLF output and content, non-ASCII paths, worktrees, drive roots,
      and UNC paths supported by the surrounding tools.

## 7. Audit Git platform conventions

Primary files: `src/git.rs` and `scripts/update.py`.

- [ ] Test `/dev/null` usage for empty-tree hashing and untracked-file
      diffs with Git for Windows. Replace with a portable mechanism where
      needed; preserve correct behavior on remote Unix machines.
- [ ] Verify the updater's `core.hooksPath=/dev/null` convention. Use a
      portable disabled-hooks configuration if required.
- [ ] Test all comparison modes, including an initial commit, repositories
      without commits, untracked files, and branch merge-base selection.
- [ ] Confirm reads still avoid repository mutation, external diff
      drivers, textconv, staging, and unexpected hooks.

## 8. Port updates without weakening safety

Primary files: `scripts/update.py`, `mods/annotator/updates.js`, and
`mods/annotator/main.js`.

- [ ] Replace unconditional `fcntl` import/use with platform-specific or
      cross-platform exclusive locking.
- [ ] Replace Unix `pass_fds`, inherited-lock assumptions, and session
      detachment with a Windows worker-launch and lock-ownership protocol.
- [ ] Ensure there is no unlocked handoff window allowing two installers
      to run. Validate ownership using durable job identity and live locks,
      not only a stored PID or status field.
- [ ] Preserve worker survival across Herdr restarts, log capture, bounded
      subprocess execution, and reliable stale-job recovery.
- [ ] Use the recorded Python interpreter and platform-aware paths.
- [ ] Preserve release tag/commit checks, source cleanliness and ancestry
      checks, file digest verification, backups, and explicit user consent.
- [ ] Test installer failure, worker crash, app restart, concurrent update
      attempts, sharing violations, and successful post-install verification.
- [ ] Keep review editors gated during real installations and uncertain
      update status. Never unblock merely because a status command failed.
- [ ] For an initial release without Windows updates, explicitly mark the
      feature unsupported and bypass updater startup gating safely. Merely
      ignoring errors is insufficient: the current gate stays closed until
      status succeeds. Provide manual reinstall instructions instead.

## 9. Make tests and CI portable

Primary files: `.github/workflows/check.yml`, `tests/ordering.rs`,
`tests/github.rs`, `tests/reviews.rs`, `tests/test_install.py`,
`tests/test_updates.py`, and subprocess unit tests in `src/process.rs`.

- [ ] Add `windows-latest` alongside Linux and macOS.
- [ ] Set up Python explicitly and use a verified interpreter command.
- [ ] Replace shared shebang/permission-based fake executables with
      portable test helpers or platform-specific equivalents.
- [ ] Keep genuinely Unix-only fork, signal, permission, and symlink tests
      gated; add Windows equivalents for the guarantees they protect.
- [ ] Test child trees, inherited output pipes, endless and finite output
      floods, hanging children, timeout diagnostics, and bounded cleanup.
- [ ] Add installer/updater tests for `.exe` names, recorded Python paths,
      Windows locking, worker launch, UTF-8, and failure recovery.
- [ ] Add path tests for drive letters, UNC paths, separators, containment,
      canonical prefixes, Unicode, and local/remote distinctions.
- [ ] Run formatting, Clippy across all targets, locked Rust tests,
      JavaScript tests, Python compilation, and Python unit tests.
- [ ] Keep macOS-only screenshot tooling optional; port development
      fixtures only where needed for Windows validation.

## 10. Validate in native Herdr and document support

- [ ] Check the installed mods with `herdr-app check`; remember this only
      checks initial rendering, not deferred rows or async states.
- [ ] Exercise sidebar and full review, unified and split diffs, context
      expansion, syntax highlighting, resizing, selection, and copying.
- [ ] Exercise Ctrl+Enter, comment drafts, editing/deletion, persistence,
      re-anchoring, exports, and sent/resolved history.
- [ ] Validate source-agent targeting and delivery to idle/working agents,
      permission prompts, uncertainty handling, and explicit retries.
- [ ] Validate GitHub reads, polling, reply drafts, explicit posting, and
      uncertain-post recovery using a disposable test PR.
- [ ] Validate optional AI ordering with supported native installations of
      both runners, without altering source chat configuration.
- [ ] Validate local Windows and supported SSH checkouts, including remote
      failures and disconnected machines.
- [ ] Test restricted desktop PATH environments, reinstall, app restart,
      helper concurrency, and updater recovery.
- [ ] Update `README.md`, `docs/guide.md`, `docs/releases.md`, and `RECIPE.md`
      with supported Windows versions, prerequisites, install commands,
      paths, optional tools, and any feature limitations.
- [ ] Publish Windows support only for features covered by passing native
      tests; state remaining limitations explicitly.

## Delivery milestones

### Milestone A: usable native Windows reviews

- Portable helper processes, executable discovery, installation, storage,
  and core path handling.
- Diff browsing, saved comments, export, and source-agent delivery work.
- Native `gh` integration validated where available.
- Updates explicitly disabled without blocking editing if the safe
  Windows updater is not ready.
- Windows CI covers core portable behavior and native Herdr smoke tests pass.

### Milestone B: full feature parity

- Safe Windows updater, including locking, worker survival, and recovery.
- OpenCode/Pi ordering and source-model matching validated.
- Open-file API and supported Windows path edge cases covered.
- Documentation and the Linux/macOS/Windows test matrix complete.

## Rough effort and main risks

Source-review estimate for a developer familiar with this project and with
access to Windows: approximately 3–5 engineering days for Milestone A and
1–2 weeks total for reliable parity. Re-estimate after baseline testing.

The highest-risk work is preserving bounded subprocess capture/cancellation,
race-free update ownership, Windows executable replacement, and delivery
uncertainty guarantees. Do not trade those guarantees for a successful build.
