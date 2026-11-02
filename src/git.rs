use crate::{
    ChangedFile, Comparison, DiffLine, DiffMode, Result, Scope, Snapshot, ViewSpec, model::hash,
};
use serde_json::Value;
use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::Duration,
};
use wait_timeout::ChildExt;

const MAX_OUTPUT: usize = 512 * 1024;
const MAX_ROWS: usize = 2000;

pub(crate) struct Output {
    pub code: i32,
    pub text: String,
    pub truncated: bool,
}

fn drain(mut reader: impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut out = Vec::new();
    let mut buf = [0; 8192];
    let mut truncated = false;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let keep = n.min(limit.saturating_sub(out.len()));
        out.extend_from_slice(&buf[..keep]);
        truncated |= keep != n;
    }
    Ok((out, truncated))
}

pub(crate) fn run(mut command: Command) -> Result<Output> {
    run_limit(&mut command, MAX_OUTPUT)
}

fn run_limit(command: &mut Command, limit: usize) -> Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Cannot start command: {e}"))?;
    let stdout = child.stdout.take().ok_or("Missing stdout")?;
    let stderr = child.stderr.take().ok_or("Missing stderr")?;
    let out = thread::spawn(move || drain(stdout, limit));
    let err = thread::spawn(move || drain(stderr, 8192));
    let status = child
        .wait_timeout(Duration::from_secs(60))
        .map_err(|e| e.to_string())?;
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        // Don't join readers: an unrelated grandchild may have inherited a pipe.
        return Err("Command timed out after 60 seconds".into());
    }
    let (bytes, truncated) = out
        .join()
        .map_err(|_| "stdout reader failed")?
        .map_err(|e| e.to_string())?;
    let (errors, _) = err
        .join()
        .map_err(|_| "stderr reader failed")?
        .map_err(|e| e.to_string())?;
    let code = status.and_then(|s| s.code()).unwrap_or(-1);
    let text = String::from_utf8(bytes)
        .map_err(|_| "Non-UTF-8 output is not supported in this first version")?;
    if code != 0 && code != 1 {
        return Err(format!(
            "Command failed ({code}): {}",
            String::from_utf8_lossy(&errors).trim()
        ));
    }
    Ok(Output {
        code,
        text,
        truncated,
    })
}

fn hc() -> Command {
    Command::new(std::env::var_os("HERDR_ANNOTATOR_HC").unwrap_or_else(|| "hc".into()))
}

pub(crate) fn git(scope: &Scope, args: &[&str]) -> Result<Output> {
    let mut command = if let Some(machine) = &scope.machine {
        let mut command = hc();
        command.args([
            "exec",
            "--on",
            machine,
            "--timeout",
            "45",
            "--env",
            "GIT_TERMINAL_PROMPT=0",
            "git",
        ]);
        command
    } else {
        Command::new("git")
    };
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0");
    command
        .args([
            "--literal-pathspecs",
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "diff.autoRefreshIndex=false",
            "-C",
            &scope.repo,
        ])
        .args(args);
    let metadata = args.contains(&"--name-status")
        || args.first() == Some(&"status")
        || args.first() == Some(&"for-each-ref");
    run_limit(
        &mut command,
        if metadata {
            8 * 1024 * 1024
        } else {
            MAX_OUTPUT
        },
    )
}

pub(crate) fn required(scope: &Scope, args: &[&str]) -> Result<String> {
    let out = git(scope, args)?;
    if out.code != 0 || out.truncated {
        return Err(format!(
            "Git query failed or exceeded its size limit: {}",
            args.first().unwrap_or(&"git")
        ));
    }
    Ok(out.text)
}

pub(crate) fn canonical_scope(scope: &Scope) -> Result<Scope> {
    if scope.repo.is_empty() || scope.repo.contains('\0') {
        return Err("A checkout folder is required".into());
    }
    if scope.machine.as_ref().is_some_and(|m| {
        m.is_empty()
            || m.starts_with('-')
            || !m
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    }) {
        return Err("Invalid herdr-core machine name".into());
    }
    let root = required(scope, &["rev-parse", "--show-toplevel"])?;
    Ok(Scope {
        repo: root.trim_end_matches('\n').to_owned(),
        machine: scope.machine.clone(),
        comparison: scope.comparison.clone(),
    })
}

pub fn snapshot(scope: &Scope, view: &ViewSpec) -> Result<Snapshot> {
    if view.context > 1_000_000 {
        return Err("Context must be between 0 and 1000000 lines".into());
    }
    let mut scope = canonical_scope(scope)?;
    let head_result = git(&scope, &["rev-parse", "--verify", "--quiet", "HEAD"])?;
    let head = (head_result.code == 0).then(|| head_result.text.trim().to_owned());
    let branch_result = git(&scope, &["symbolic-ref", "--short", "-q", "HEAD"])?;
    let branch = (branch_result.code == 0).then(|| branch_result.text.trim().to_owned());
    let bases: Vec<String> = required(
        &scope,
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads",
            "refs/remotes",
        ],
    )?
    .lines()
    .filter(|s| !s.ends_with("/HEAD"))
    .map(str::to_owned)
    .collect();
    let merge_base = if scope.comparison.mode == DiffMode::Branch {
        if head.is_none() {
            return Err("Branch scope needs a commit and a base branch".into());
        }
        let base = if let Some(base) = &scope.comparison.base {
            base.clone()
        } else if bases.iter().any(|base| base == "develop") {
            // Review local integration work, not an old remote-tracking ref.
            // This also applies on develop itself: only worktree changes remain.
            "develop".to_owned()
        } else {
            let remote_head = git(
                &scope,
                &[
                    "symbolic-ref",
                    "--quiet",
                    "--short",
                    "refs/remotes/origin/HEAD",
                ],
            )?;
            let default = (remote_head.code == 0).then(|| remote_head.text.trim().to_owned());
            default
                .filter(|s| bases.contains(s))
                .or_else(|| {
                    [
                        "origin/develop",
                        "origin/main",
                        "origin/master",
                        "develop",
                        "main",
                        "master",
                    ]
                    .into_iter()
                    .find(|s| bases.iter().any(|b| b == s) && branch.as_deref() != Some(s))
                    .map(str::to_owned)
                })
                .ok_or("Choose a base branch; no default base is available locally")?
        };
        let commit = required(
            &scope,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{base}^{{commit}}"),
            ],
        )
        .map_err(|_| {
            format!(
                "Base '{base}' is not available locally; choose a local ref (no automatic fetch)"
            )
        })?;
        let merge = required(&scope, &["merge-base", commit.trim(), "HEAD"])
            .map_err(|_| "Base and HEAD have no common ancestor")?;
        scope.comparison.base = Some(base);
        Some(merge.trim().to_owned())
    } else {
        scope.comparison = Comparison::default();
        None
    };
    let status = required(
        &scope,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--no-renames",
            "--untracked-files=all",
        ],
    )?;
    let baseline = merge_base.as_deref().or(head.as_deref());
    let mut entries = std::collections::BTreeMap::<String, String>::new();
    if let Some(base) = baseline {
        let changed = required(
            &scope,
            &[
                "diff",
                "--name-status",
                "-z",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
                base,
                "--",
            ],
        )?;
        let mut records = changed.split('\0').filter(|s| !s.is_empty());
        while let Some(status) = records.next() {
            let path = records.next().ok_or("Invalid Git name/status output")?;
            entries.insert(path.to_owned(), status.to_owned());
        }
    }
    for entry in status.split('\0').filter(|s| !s.is_empty()) {
        if entry.len() < 4 || !entry.is_char_boundary(3) {
            return Err("Invalid Git status record".into());
        }
        if baseline.is_none() || entry.starts_with("?? ") {
            entries.insert(entry[3..].to_owned(), entry[..2].to_owned());
        }
    }
    let limited = false;
    let mut files = Vec::new();
    for (path, xy) in &entries {
        if !safe_path(path) {
            return Err("Git returned an invalid relative path".into());
        }
        let file = ChangedFile {
            path: path.to_owned(),
            status: xy.to_owned(),
            rows: Vec::new(),
            note: None,
            truncated: false,
            loaded: false,
        };
        files.push(file);
    }
    let selected = view
        .file
        .as_ref()
        .filter(|path| files.iter().any(|f| &f.path == *path))
        .cloned()
        .or_else(|| files.first().map(|f| f.path.clone()));
    let mut snapshot = Snapshot {
        scope,
        branch,
        head,
        revision: String::new(),
        files,
        limited,
        bases,
        merge_base,
        view: ViewSpec {
            file: selected.clone(),
            context: view.context,
        },
    };
    if let Some(path) = selected {
        hydrate(&mut snapshot, &[path])?;
    }
    seal(&mut snapshot)?;
    Ok(snapshot)
}

pub(crate) fn hydrate(snapshot: &mut Snapshot, paths: &[String]) -> Result<()> {
    let scope = &snapshot.scope;
    let baseline = snapshot.merge_base.as_deref().or(snapshot.head.as_deref());
    let unified = format!("--unified={}", snapshot.view.context);
    let mut budget = (2 * 1024 * 1024usize).saturating_sub(
        snapshot
            .files
            .iter()
            .flat_map(|f| &f.rows)
            .map(|r| r.text.len())
            .sum::<usize>(),
    );
    for file in &mut snapshot.files {
        if file.loaded || !paths.contains(&file.path) {
            continue;
        }
        file.loaded = true;
        if budget == 0 {
            file.truncated = true;
            file.note = Some("Preview budget reached; select this file to load it first".into());
            continue;
        }
        let path = file.path.as_str();
        // No external diff drivers, textconv, index updates, staging or private Git refs.
        let args = if file.status == "??" || baseline.is_none() {
            vec![
                "diff",
                "--no-index",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                &unified,
                "--",
                "/dev/null",
                path,
            ]
        } else {
            vec![
                "diff",
                baseline.ok_or("Missing comparison baseline")?,
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--no-renames",
                &unified,
                "--",
                path,
            ]
        };
        let out = git(scope, &args)?;
        if out.code != 0 && !(args.contains(&"--no-index") && out.code == 1) {
            return Err(format!(
                "Cannot load diff for {}: {}",
                file.path,
                out.text.trim()
            ));
        }
        let too_wide = out.text.lines().any(|line| line.len() > 8192);
        if out.truncated || out.text.len() > budget || too_wide {
            file.truncated = true;
            file.note = Some("Diff exceeds preview limits; file notes only".into());
            budget = budget.saturating_sub(out.text.len());
        } else {
            budget = budget.saturating_sub(out.text.len());
            file.rows = parse_diff(&out.text)?;
            if file.rows.len() > MAX_ROWS {
                file.rows.truncate(MAX_ROWS);
                file.truncated = true;
                file.note = Some("Only the first 2000 rows are shown; file notes only".into());
            }
            if file.rows.is_empty() {
                file.note = Some("Binary, mode-only, submodule, or no text diff".into());
            }
        }
    }
    Ok(())
}

pub(crate) fn seal(snapshot: &mut Snapshot) -> Result<()> {
    snapshot.revision.clear();
    snapshot.revision = hash(&serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?);
    Ok(())
}

// Anchor validation retains the context in which each note was written, even
// when the visible preview is collapsed. It never changes the visible rows.
pub(crate) fn validation(snapshot: &Snapshot, comments: &[crate::Comment]) -> Result<Snapshot> {
    let mut result = snapshot.clone();
    let mut contexts = std::collections::BTreeMap::<String, u32>::new();
    for comment in comments {
        if comment.anchor.lines.is_some() && comment.branch == snapshot.branch {
            let context = contexts.entry(comment.path.clone()).or_default();
            *context = (*context).max(comment.context.min(1_000_000));
        }
    }
    for (path, context) in contexts {
        if context <= snapshot.view.context {
            continue;
        }
        let Some(index) = result.files.iter().position(|f| f.path == path) else {
            continue;
        };
        let mut expanded = snapshot.clone();
        expanded.files = vec![snapshot.files[index].clone()];
        expanded.files[0].rows.clear();
        expanded.files[0].loaded = false;
        expanded.files[0].truncated = false;
        expanded.files[0].note = None;
        expanded.view.context = context;
        hydrate(&mut expanded, &[path])?;
        result.files[index] = expanded.files.remove(0);
    }
    Ok(result)
}

fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.split('/').any(|p| p == ".." || p.is_empty())
        && !path.contains('\0')
}

pub fn parse_diff(patch: &str) -> Result<Vec<DiffLine>> {
    let mut rows = Vec::new();
    let mut old = 0u32;
    let mut new = 0u32;
    let mut hunk = false;
    for line in patch.lines() {
        if line.starts_with("@@ ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let parse = |s: &str| -> Result<u32> {
                s.get(1..)
                    .and_then(|s| s.split(',').next())
                    .ok_or("Invalid diff header")?
                    .parse()
                    .map_err(|_| "Invalid diff line number".into())
            };
            old = parse(parts.get(1).ok_or("Missing old hunk range")?)?;
            new = parse(parts.get(2).ok_or("Missing new hunk range")?)?;
            hunk = true;
            rows.push(DiffLine {
                kind: "hunk".into(),
                old: None,
                new: None,
                text: line.to_owned(),
            });
            continue;
        }
        if !hunk {
            continue;
        }
        let mut row = DiffLine {
            kind: "meta".into(),
            old: None,
            new: None,
            text: line.to_owned(),
        };
        match line.as_bytes().first() {
            Some(b'+') => {
                row.kind = "add".into();
                row.new = Some(new);
                new = new.checked_add(1).ok_or("Line number overflow")?;
                row.text = line[1..].to_owned();
            }
            Some(b'-') => {
                row.kind = "delete".into();
                row.old = Some(old);
                old = old.checked_add(1).ok_or("Line number overflow")?;
                row.text = line[1..].to_owned();
            }
            Some(b' ') => {
                row.kind = "context".into();
                row.old = Some(old);
                row.new = Some(new);
                old = old.checked_add(1).ok_or("Line number overflow")?;
                new = new.checked_add(1).ok_or("Line number overflow")?;
                row.text = line[1..].to_owned();
            }
            _ => (),
        }
        rows.push(row);
    }
    Ok(rows)
}

pub fn validate_target(scope: &Scope, target: &str) -> Result<()> {
    let (machine, address) = match target.split_once('/') {
        Some((m, a)) => (Some(m), a),
        None => (None, target),
    };
    if machine != scope.machine.as_deref()
        || address.len() < 2
        || !address.starts_with('t')
        || !address[1..].bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(
            "Choose a terminal ID on the reviewed machine, not a name or another machine".into(),
        );
    }
    let mut command = hc();
    command.args(["get", target]);
    let out = run(command)?;
    if out.code != 0 || out.truncated {
        return Err("Cannot inspect target agent".into());
    }
    let terminal: Value = serde_json::from_str(&out.text).map_err(|e| e.to_string())?;
    if terminal["state"] != "live"
        || terminal["status"]["state"] != "idle"
        || terminal["status"]["app"].as_str().is_none_or(str::is_empty)
    {
        return Err("Target must be a live, reported, idle agent; blocked/working/unreported terminals are refused".into());
    }
    let cwd = terminal["foreground"]["cwd"]
        .as_str()
        .or_else(|| terminal["cwd"].as_str())
        .ok_or("Agent has no working folder")?;
    let target_scope = Scope {
        repo: cwd.to_owned(),
        machine: scope.machine.clone(),
        comparison: Comparison::default(),
    };
    let root = required(&target_scope, &["rev-parse", "--show-toplevel"])?;
    if root.trim_end_matches('\n') != scope.repo {
        return Err("Target agent is in another worktree".into());
    }
    Ok(())
}

pub fn send(target: &str, text: &str) -> Result<()> {
    let mut command = hc();
    command.args(["send", "--if", "idle", target, text]);
    let out = run(command)?;
    if out.code != 0 {
        return Err("herdr-core refused delivery".into());
    }
    Ok(())
}
