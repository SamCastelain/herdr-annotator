# Native Windows support plan

## Goal and scope

Run Herdr Annotator in native Windows Herdr App, against local Windows
checkouts and supported SSH checkouts, without requiring WSL or Git Bash.
Preserve existing macOS and Linux behavior.

Herdr's local manual (`~/.config/herdr-app/AGENTS.md`) already documents
Windows support for the host UI, terminals, file watching, and commands.
The main work is in Annotator's helper, installation, update machinery,
and path handling—not a rewrite of the GPUI interface.

Windows and WSL now have independent checkouts, synchronized through a shared
GitHub fork. The user can run tests on both sides; Rust and Python currently
fail on Windows. Native Herdr behavior and Windows configuration paths still
need verification.

## 1. Establish a native Windows baseline

- [x] Set up independent Windows and WSL checkouts with a shared GitHub fork.
- [x] Provision native Windows tooling sufficient to invoke the Rust,
      Python, and JavaScript test suites (user-reported).
- [ ] Verify Windows Herdr App, herdr-core (`hc`), tool versions, and native
      desktop execution independently of the test tooling.
- [ ] Install and authenticate `gh` for optional GitHub integration tests.
- [ ] Record Windows Herdr's actual app-home location, `home()` and
      `appHome()` values, command discovery, and `exec` behavior.
- [ ] Record the Windows Rust target/toolchain and build prerequisites.
- [ ] Inventory native executable and npm launcher forms for optional AI
      runners; do not assume `.cmd` files execute like Unix scripts.
- [x] Run the existing checks on both sides before implementation. The user
      reports Windows Rust/Python failures; the WSL Rust, Python, and
      JavaScript suites pass.
- [ ] Capture exact Windows failures and classify compile errors separately
      from test fixture/runtime failures.

## 2. Add a Windows subprocess backend

Primary files: `src/process.rs`, `src/process/unix.rs`, and
`src/process/windows.rs`. Implementation and native Windows test validation
are complete, with the real-executable requirement documented below.

- [x] Separate shared request deadlines, output limits, result types, and
      diagnostics from platform-specific process and pipe management.
- [x] Keep Unix process groups, signals, `fcntl`, and `poll` behind Unix
      configuration gates.
- [x] Implement Windows child lifetime management, preferably with Job
      Objects, including a safe strategy for assigning spawned processes
      before descendants can escape management.
- [x] Capture stdout and stderr concurrently with bounded retention and
      fair draining, even when output exceeds retention limits.
- [x] Ensure inherited pipe handles cannot extend capture indefinitely
      after the direct child exits. Do not introduce indefinitely blocked
      reader threads or cleanup waits.
- [x] Preserve shared 25-second request budgets, 120-second generation
      budgets, and the bounded post-exit drain period.
- [x] Preserve diagnostics on timeout, strict UTF-8 stdout handling, lossy
      stderr decoding, truncation flags, and nonzero exit handling.
- [x] Bound cleanup for errors, timeouts, and unwinding; close owned handles.
- [x] Avoid visible console windows for background helper commands where
      appropriate, without breaking core or runner invocation.
- [x] Preserve the rule that local cancellation does not prove remote
      delivery was cancelled or never happened.
- [x] Scope any new Windows dependencies in `Cargo.toml` appropriately.
- [x] Run the subprocess and storage unit tests natively on Windows and
      relay failures before treating these runtime guarantees as verified.

## 3. Make executable discovery and invocation portable

Primary files: `src/executables.rs`, `src/git.rs`, `src/github.rs`,
`src/ordering.rs`, and `src/ordering_models.rs`.

- [x] Replace unconditional Unix permission checks with platform-specific
      executable validation.
- [x] Resolve the user's home on Windows without requiring `HOME`.
- [x] Search Windows executable extensions and appropriate user install
      locations while retaining explicit environment overrides.
- [x] Keep installer-recorded executable paths as the preferred discovery
      source and validate receipt paths before use.
- [x] Audit direct `git`, `gh`, and `hc` launches for consistent discovery
      from a desktop environment with a restricted PATH.
- [x] Handle npm `.cmd` launchers explicitly and safely. Prefer invoking
      Node and a resolved script entry point where practical; do not build
      shell command strings containing review content.
- [ ] Verify OpenCode and Pi runner invocation, including PATH passed
      through local `hc exec` and discovery of Pi's Node runtime.
- [ ] Test paths and arguments containing spaces, parentheses,
      apostrophes, Unicode, and shell metacharacters.
- [x] Retain the existing policy of not sourcing interactive shell files
      merely to discover tools.

Implementation notes for increment 3:

- Local Git, GitHub CLI, and all core launches use shared discovery and runtime
  PATH; `HERDR_ANNOTATOR_GIT` is available alongside the existing overrides.
  Remote Git continues to resolve on the target machine.
- Added Windows Git/GitHub CLI install locations, pnpm and Scoop directories.
- Known npm runner shims resolve through package manifests to contained entry
  points, invoked with Node and literal argv. No batch text is executed or
  parsed. Unsupported batch launchers fail closed, including Node overrides.
- Pi forwards the resolved program and entry-point arguments through local
  `hc exec`, with the augmented PATH. Native runner validation remains open;
  Unix ordering regression fixtures still exercise the existing invocation.
- Root-commit empty-tree hashing reads closed stdin rather than a host-specific
  null-device filename. Git no-index diff's `/dev/null` sentinel is retained.
- Added isolated discovery precedence/invalid-receipt tests and npm manifest
  tests with spaces, punctuation and Unicode in installation paths. Native
  Windows argument/runner validation remains a separate gate.
- Assistant-run WSL validation: all 96 Rust tests pass (29 library, 13 GitHub,
  9 ordering, 45 reviews); Linux and Windows-target Clippy pass with warnings
  denied. No native Windows run was performed by the assistant. Handoff:
  retain the real Python override and run `cargo test --locked` and
  `cargo clippy --locked --all-targets -- -D warnings` in native PowerShell.

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

- [x] Add a Windows default state directory without requiring `HOME`.
      Preserve `HERDR_ANNOTATOR_STATE_DIR` and existing Unix conventions.
- [ ] Keep Unix file modes behind Unix configuration gates and document
      the Windows permissions model used for private review data.
- [x] Gate directory syncing in `write_atomic` by platform or use a
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

### Increment 1: portability foundations

- Implemented shared home/state resolution in `src/platform.rs`. Windows
  state uses `%LOCALAPPDATA%/herdr-annotator`, falling back to the profile's
  `AppData/Local/herdr-annotator`; an explicit state override wins.
- Installer receipts remain at the existing home-relative
  `.local/share/herdr-annotator/install.json` location until installation
  paths are ported together with the installer.
- Executable discovery now gates Unix permissions, searches native `.exe`
  and `.com` candidates on Windows, and adds Windows tool directories.
  npm/batch launcher invocation and full discovery validation remain open.
- Atomic state writes close the temporary file before replacement and sync
  directory entries only on Unix. Windows uses inherited filesystem ACLs;
  this does not add Unix-style private modes or a Windows directory-durability
  guarantee.
- Platform selection uses `#[cfg(windows)]` / `#[cfg(unix)]` implementations;
  platform-specific tests run on their native platform, not through runtime
  platform booleans. Fake environment lookups keep tests isolated.
- Added environment-independent path tests and platform-portable storage,
  locking, and executable validation tests. Native Windows verification is
  still pending. At the end of this increment, the Unix-only subprocess
  backend and test fixtures still prevented Windows Rust compilation;
  increment 2 addresses that compilation barrier.
- WSL validation: all 88 Rust tests pass (8 new on Unix), as do formatting and
  Clippy with warnings denied. The unchanged JavaScript and Python suites
  passed the baseline run (136 and 17 tests respectively).

### Increment 2: Windows subprocess backend

- Shared deadlines, capture limits, diagnostics, and result handling stay in
  `src/process.rs`; OS-specific lifetime, pipe reads, and waiting live in
  `src/process/unix.rs` and `src/process/windows.rs`.
- Windows starts each child suspended and assigns a non-inherited Job Object
  before resuming it. Stable Rust does not expose the primary thread handle,
  so documented ToolHelp APIs locate its sole thread. Ambiguous enumeration
  fails closed rather than resuming an arbitrary thread.
- The job disallows breakaway and kills descendants on cancellation or
  unwinding. Successful commands release kill-on-close, preserving the Unix
  behavior that intentional background work with closed output pipes may live.
- Windows capture uses `PeekNamedPipe` and reads only available bytes from
  solely owned pipes, retaining fair bounded draining and the existing
  250 ms post-exit drain deadline. There are no blocking reader threads.
- Added tests for literal argument handling, nested budget restoration,
  successful background work, Windows Job Object membership/unwinding, and
  descendant cleanup on timeout/inherited-pipe failure.
- Shared subprocess tests select `python` on Windows and `python3` on Unix;
  `HERDR_ANNOTATOR_TEST_PYTHON` overrides either with an explicit executable.
  Unix-only fork/signal tests and the ordering suite's shebang fixtures are
  gated; native ordering fixtures remain future work, not verified coverage.
- WSL validation: all 93 Rust tests pass. Windows-target all-target compilation
  succeeds with `cargo check --locked --all-targets --target
  x86_64-pc-windows-gnu`. Formatting and Clippy with warnings denied pass for
  the Linux and Windows targets. Cross-checks do not execute Windows code.
- Native Windows handoff: run `cargo test --locked --lib`, then
  `cargo test --locked` and
  `cargo clippy --locked --all-targets -- -D warnings`. Relay the revision,
  commands, and complete failures. Installer/updater Python failures are
  still expected until their later portability increments.
- First native Windows library run (user-reported): 25/27 tests pass.
  `timeout_preserves_diagnostics_and_bounds_cleanup` times out without its
  expected stderr marker; `job_terminates_descendants_on_timeout_and_inherited_pipe_failure`
  gets WAIT_TIMEOUT (258) waiting for descendant termination. Keep runtime
  validation open. Next diagnose serial versus parallel execution and the
  actual Python executable/launcher before changing cancellation behavior
  or relaxing test deadlines.
- Follow-up native Windows runs: using the WindowsApps Python App Execution
  Alias still fails descendant cleanup with serial tests (26/27 pass). With
  the real Python 3.15 interpreter selected by `HERDR_ANNOTATOR_TEST_PYTHON`,
  serial execution passes all 27 tests, including descendant cleanup. Parallel
  execution passes cleanup but can time out before fixture diagnostics appear.
- Python subprocess fixtures now share a test-only mutex, acquired before
  any deadline starts. Production concurrency, budgets, and cleanup assertions
  are unchanged. Rerun the normal parallel test command on Windows to verify.
- Added a deliberate two-command concurrency test within one fixture slot to
  retain coverage of independent thread-local deadlines and output capture.
  After these changes, all 94 Rust tests pass on WSL; Clippy with warnings
  denied passes for both Linux and Windows targets.
- Documented use of a real interpreter rather than an App Execution Alias in
  `docs/guide.md`. Job containment does not cover external broker activation;
  installer-recorded Python paths must use `sys.executable`, not the alias.

### Milestone A: usable native Windows reviews

Latest validation: the user reports Windows tests pass after the subprocess
fixture changes, using the real Python interpreter override. Native Windows
test validation for increment 2 is complete. This does not yet establish
installer/updater portability or native Herdr UI validation.

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
