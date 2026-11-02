use herdr_annotator::{Anchor, ChangedFile, DiffLine, Engine, Request, Scope, Selection, Side};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Fixture {
    _dir: TempDir,
    repo: std::path::PathBuf,
    state: std::path::PathBuf,
}
impl Fixture {
    fn new(commit: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "--initial-branch=main"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@example.invalid"]);
        fs::write(repo.join("file.txt"), "one\ntwo\nthree\n").unwrap();
        if commit {
            git(&repo, &["add", "."]);
            git(&repo, &["commit", "-m", "baseline"]);
        }
        let state = dir.path().join("state");
        Self {
            _dir: dir,
            repo,
            state,
        }
    }
    fn scope(&self) -> Scope {
        Scope {
            repo: self.repo.to_str().unwrap().to_owned(),
            machine: None,
            comparison: Default::default(),
        }
    }
    fn request(&self, value: Value) -> Result<Value, String> {
        let mut value = value;
        value["scope"] = json!(self.scope());
        Engine::new(self.state.clone()).handle(serde_json::from_value::<Request>(value).unwrap())
    }
    fn snapshot(&self) -> Value {
        self.request(json!({"op":"snapshot"})).unwrap()
    }
    fn branch_request(&self, mut value: Value, base: &str) -> Result<Value, String> {
        value["scope"] = json!(self.scope());
        value["scope"]["comparison"] = json!({"mode":"branch", "base":base});
        Engine::new(self.state.clone()).handle(serde_json::from_value(value).unwrap())
    }
    fn change(&self) {
        fs::write(self.repo.join("file.txt"), "one\nchanged\nthree\n").unwrap();
    }
    fn add(&self, selection: Value) -> Value {
        let s = self.snapshot();
        self.request(json!({"op":"add", "revision":s["snapshot"]["revision"], "path":"file.txt", "selection":selection, "body":"Please check this"})).unwrap()
    }
}

#[test]
fn automatic_branch_base_prefers_local_develop_over_stale_origin_and_remote_head() {
    let f = Fixture::new(true);
    git(
        &f.repo,
        &["update-ref", "refs/remotes/origin/develop", "HEAD"],
    );
    git(&f.repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(
        &f.repo,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    );
    git(&f.repo, &["switch", "-c", "develop"]);
    fs::write(f.repo.join("base-work.txt"), "local develop work\n").unwrap();
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-m", "develop advances locally"]);
    let at_develop = Engine::new(f.state.clone())
        .handle(
            serde_json::from_value(json!({
                "op":"snapshot","scope":{"repo":f.scope().repo,"comparison":{"mode":"branch"}}
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        at_develop["snapshot"]["scope"]["comparison"]["base"],
        "develop"
    );
    assert_eq!(at_develop["snapshot"]["files"].as_array().unwrap().len(), 0);
    git(&f.repo, &["switch", "-c", "feature"]);
    fs::write(f.repo.join("feature.txt"), "feature only\n").unwrap();
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-m", "feature"]);
    let value =
        json!({"op":"snapshot","scope":{"repo":f.scope().repo,"comparison":{"mode":"branch"}}});
    let automatic = Engine::new(f.state.clone())
        .handle(serde_json::from_value(value).unwrap())
        .unwrap();
    assert_eq!(
        automatic["snapshot"]["scope"]["comparison"]["base"],
        "develop"
    );
    assert_eq!(automatic["snapshot"]["files"].as_array().unwrap().len(), 1);
    assert_eq!(automatic["snapshot"]["files"][0]["path"], "feature.txt");
    let remote = f
        .branch_request(json!({"op":"snapshot"}), "origin/develop")
        .unwrap();
    assert_eq!(remote["snapshot"]["files"].as_array().unwrap().len(), 2);
    assert_eq!(
        remote["snapshot"]["scope"]["comparison"]["base"],
        "origin/develop"
    );
}

#[test]
fn snapshot_covers_staged_unstaged_untracked_and_does_not_touch_index() {
    let f = Fixture::new(true);
    f.change();
    git(&f.repo, &["add", "file.txt"]);
    fs::write(f.repo.join("file.txt"), "one\nchanged again\nthree\n").unwrap();
    fs::write(f.repo.join("odd name\tnew.txt"), "hello\n").unwrap();
    let index = fs::read(f.repo.join(".git/index")).unwrap();
    let s = f.snapshot();
    assert_eq!(s["snapshot"]["files"].as_array().unwrap().len(), 2);
    assert!(s.to_string().contains("changed again"));
    assert_eq!(fs::read(f.repo.join(".git/index")).unwrap(), index);
}

#[test]
fn unborn_repo_and_deleted_files_have_reviewable_diffs() {
    let f = Fixture::new(false);
    assert_eq!(f.snapshot()["snapshot"]["files"][0]["rows"][1]["new"], 1);
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-m", "initial"]);
    fs::remove_file(f.repo.join("file.txt")).unwrap();
    assert_eq!(f.snapshot()["snapshot"]["files"][0]["rows"][1]["old"], 1);
}

#[test]
fn line_and_file_comments_persist_across_engine_instances() {
    let f = Fixture::new(true);
    f.change();
    f.add(json!({"side":"new", "start":2, "end":2}));
    f.add(Value::Null);
    let s = f.snapshot();
    let comments = s["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0]["stale"], false);
    assert_eq!(comments[0]["position"]["start"], 2);
    assert_eq!(comments[1]["position"], Value::Null);
    let export = f
        .request(json!({"op":"export", "revision":s["snapshot"]["revision"]}))
        .unwrap();
    assert!(
        export["text"]
            .as_str()
            .unwrap()
            .contains("file.txt:2-2 (New)")
    );
    assert!(export["text"].as_str().unwrap().contains("file.txt (file)"));
}

#[test]
fn old_side_anchors_and_diff_header_like_source_lines_work() {
    let f = Fixture::new(true);
    f.change();
    let s = f.add(json!({"side":"old", "start":2, "end":2}));
    assert_eq!(s["comments"][0]["position"]["side"], "old");
    fs::write(f.repo.join("file.txt"), "one\n+++ not a header\nthree\n").unwrap();
    let s = f.snapshot();
    assert!(
        s["snapshot"]["files"][0]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["text"] == "+++ not a header" && r["kind"] == "add")
    );
}

#[test]
fn stale_snapshot_is_rejected_without_saving() {
    let f = Fixture::new(true);
    f.change();
    let s = f.snapshot();
    fs::write(f.repo.join("file.txt"), "different\n").unwrap();
    assert!(f.request(json!({"op":"add", "revision":s["snapshot"]["revision"], "path":"file.txt", "selection":null, "body":"note"})).unwrap_err().contains("Diff changed"));
    assert_eq!(f.snapshot()["comments"].as_array().unwrap().len(), 0);
}

#[test]
fn changed_or_committed_code_is_stale_and_excluded_from_feedback() {
    let f = Fixture::new(true);
    f.change();
    f.add(json!({"side":"new", "start":2, "end":2}));
    fs::write(f.repo.join("file.txt"), "one\nother\nthree\n").unwrap();
    let s = f.snapshot();
    assert_eq!(s["comments"][0]["stale"], true);
    assert!(
        f.request(json!({"op":"export", "revision":s["snapshot"]["revision"]}))
            .is_err()
    );
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-m", "done"]);
    assert_eq!(f.snapshot()["comments"][0]["stale"], true);
}

#[test]
fn branch_switch_never_silently_reuses_notes() {
    let f = Fixture::new(true);
    f.change();
    f.add(Value::Null);
    git(&f.repo, &["switch", "-c", "other"]);
    assert_eq!(f.snapshot()["comments"][0]["stale"], true);
}

#[test]
fn resolve_and_reopen_control_feedback() {
    let f = Fixture::new(true);
    f.change();
    let s = f.add(Value::Null);
    let id = &s["comments"][0]["id"];
    let resolved = f
        .request(json!({"op":"resolve", "id":id, "resolved":true}))
        .unwrap();
    assert!(
        f.request(json!({"op":"export", "revision":resolved["snapshot"]["revision"]}))
            .is_err()
    );
    f.request(json!({"op":"resolve", "id":id, "resolved":false}))
        .unwrap();
    assert_eq!(f.snapshot()["comments"][0]["resolved"], false);
}

#[test]
fn invalid_ranges_and_file_paths_cannot_create_notes() {
    let f = Fixture::new(true);
    f.change();
    let s = f.snapshot();
    for selection in [
        json!({"side":"new","start":0,"end":2}),
        json!({"side":"new","start":2,"end":201}),
        json!({"side":"new","start":2,"end":99}),
    ] {
        assert!(f.request(json!({"op":"add", "revision":s["snapshot"]["revision"], "path":"file.txt", "selection":selection, "body":"note"})).is_err());
    }
    assert!(f.request(json!({"op":"add", "revision":s["snapshot"]["revision"], "path":"../secret", "selection":null, "body":"note"})).is_err());
}

#[test]
fn corrupt_stores_are_not_overwritten() {
    let f = Fixture::new(true);
    f.change();
    f.add(Value::Null);
    let path = fs::read_dir(&f.state)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "json"))
        .unwrap();
    fs::write(&path, "broken").unwrap();
    assert!(
        f.request(json!({"op":"snapshot"}))
            .unwrap_err()
            .contains("not overwritten")
    );
    assert_eq!(fs::read_to_string(path).unwrap(), "broken");
}

fn file(lines: &[&str], first: u32) -> ChangedFile {
    ChangedFile {
        path: "a".into(),
        status: " M".into(),
        note: None,
        truncated: false,
        loaded: true,
        rows: lines
            .iter()
            .enumerate()
            .map(|(i, s)| DiffLine {
                kind: "add".into(),
                old: None,
                new: Some(first + i as u32),
                text: (*s).into(),
            })
            .collect(),
    }
}

#[test]
fn anchors_move_only_to_unique_exact_context() {
    let anchor = Anchor::create(
        &file(&["before", "target", "after"], 1),
        Some(Selection {
            side: Side::New,
            start: 2,
            end: 2,
        }),
    )
    .unwrap();
    let moved = anchor
        .locate(&file(&["before", "target", "after"], 10))
        .unwrap()
        .unwrap();
    assert_eq!(moved.start, 11);
    assert!(
        anchor
            .locate(&file(
                &["before", "target", "after", "before", "target", "after"],
                1
            ))
            .is_none()
    );
    assert!(
        anchor
            .locate(&file(&["before", "changed", "after"], 1))
            .is_none()
    );
}

#[test]
fn oversized_files_and_binary_files_stay_visible() {
    let f = Fixture::new(true);
    fs::write(f.repo.join("big.txt"), "x".repeat(9000)).unwrap();
    fs::write(f.repo.join("binary.bin"), [0, 1, 2, 3]).unwrap();
    let s = f.snapshot();
    let files = s["snapshot"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert!(
        files
            .iter()
            .filter(|f| f["loaded"] == true)
            .all(|f| f["note"].is_string())
    );
    let binary = f
        .request(json!({"op":"snapshot","view":{"file":"binary.bin"}}))
        .unwrap();
    assert!(
        binary["snapshot"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == "binary.bin")
            .unwrap()["note"]
            .is_string()
    );
    assert_eq!(
        files.iter().find(|f| f["path"] == "big.txt").unwrap()["truncated"],
        true
    );
}

#[test]
fn invalid_machine_is_rejected_before_running_a_command() {
    let f = Fixture::new(true);
    let request = Request::Snapshot {
        view: Default::default(),
        scope: Scope {
            repo: f.scope().repo,
            machine: Some("--bad".into()),
            comparison: Default::default(),
        },
    };
    assert!(
        Engine::new(f.state)
            .handle(request)
            .unwrap_err()
            .contains("Invalid")
    );
}

#[test]
fn cannot_send_to_another_machine_and_notes_remain_unsent() {
    let f = Fixture::new(true);
    f.change();
    let s = f.add(Value::Null);
    assert!(
        f.request(json!({"op":"send", "revision":s["snapshot"]["revision"], "target":"other/t1"}))
            .unwrap_err()
            .contains("reviewed machine")
    );
    assert_eq!(f.snapshot()["comments"][0]["delivery"], Value::Null);
}

#[cfg(unix)]
#[test]
fn notes_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new(true);
    f.change();
    f.add(Value::Null);
    for entry in fs::read_dir(f.state).unwrap() {
        assert_eq!(
            entry.unwrap().metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn literal_git_pathspecs_do_not_mix_up_bracketed_filenames() {
    let f = Fixture::new(true);
    fs::write(f.repo.join("[a].txt"), "bracket baseline\n").unwrap();
    fs::write(f.repo.join("a.txt"), "other baseline\n").unwrap();
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-m", "paths"]);
    fs::write(f.repo.join("[a].txt"), "bracket changed\n").unwrap();
    fs::write(f.repo.join("a.txt"), "other changed\n").unwrap();
    let s = f.snapshot();
    let file = s["snapshot"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["path"] == "[a].txt")
        .unwrap();
    assert!(file.to_string().contains("bracket changed"));
    assert!(!file.to_string().contains("other changed"));
}

#[test]
fn simultaneous_writers_keep_both_notes() {
    let f = Fixture::new(true);
    f.change();
    let s = f.snapshot();
    let scope = f.scope();
    let state = f.state.clone();
    let revision = s["snapshot"]["revision"].as_str().unwrap().to_owned();
    let threads: Vec<_> = (0..2)
        .map(|i| {
            let scope = scope.clone();
            let state = state.clone();
            let revision = revision.clone();
            std::thread::spawn(move || {
                Engine::new(state)
                    .handle(Request::Add {
                        view: Default::default(),
                        scope,
                        revision,
                        path: "file.txt".into(),
                        selection: None,
                        body: format!("note {i}"),
                    })
                    .unwrap()
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(f.snapshot()["comments"].as_array().unwrap().len(), 2);
}

#[test]
fn worktree_notes_are_isolated() {
    let f = Fixture::new(true);
    f.change();
    f.add(Value::Null);
    let second = f._dir.path().join("other");
    git(
        &f.repo,
        &["worktree", "add", "--detach", second.to_str().unwrap()],
    );
    fs::write(second.join("file.txt"), "second checkout\n").unwrap();
    let s = Engine::new(f.state.clone())
        .handle(Request::Snapshot {
            view: Default::default(),
            scope: Scope {
                repo: second.to_str().unwrap().into(),
                machine: None,
                comparison: Default::default(),
            },
        })
        .unwrap();
    assert_eq!(s["comments"].as_array().unwrap().len(), 0);
}

#[cfg(unix)]
#[test]
fn untracked_symlinks_show_the_target_path_not_its_contents() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new(true);
    let outside = f._dir.path().join("outside");
    fs::write(&outside, "SECRET-CONTENT\n").unwrap();
    symlink(&outside, f.repo.join("link")).unwrap();
    let s = f.snapshot();
    assert!(!s.to_string().contains("SECRET-CONTENT"));
    assert!(
        s["snapshot"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["path"] == "link")
    );
}

// Run the actual CLI against a fake hc. Never creates a terminal or sends to a real agent.
#[cfg(unix)]
struct FakeCore {
    fixture: Fixture,
    program: std::path::PathBuf,
    log: std::path::PathBuf,
}
#[cfg(unix)]
impl FakeCore {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new(true);
        fixture.change();
        let program = fixture._dir.path().join("hc");
        let log = fixture._dir.path().join("calls.jsonl");
        fs::write(&program, r#"#!/usr/bin/env python3
import json,os,subprocess,sys
args=sys.argv[1:]
with open(os.environ['FAKE_LOG'],'a') as f: f.write(json.dumps(args)+'\n')
if args[0]=='exec':
    p=subprocess.run(args[args.index('git'):]); sys.exit(p.returncode)
if args[0]=='get':
    print(json.dumps({'state':'live','status':{'state':os.environ.get('FAKE_STATUS','idle'),'app':'opencode'},'foreground':{'cwd':os.environ['FAKE_CWD']}})); sys.exit(0)
if args[0]=='send':
    if os.environ.get('FAKE_FAIL_SEND')=='1': print('disconnected',file=sys.stderr); sys.exit(2)
    print(json.dumps({'turn':1,'started':True})); sys.exit(0)
sys.exit(2)
"#).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            fixture,
            program,
            log,
        }
    }
    fn request(&self, mut value: Value, status: &str, fail_send: bool) -> Value {
        value["scope"] = json!({"repo":self.fixture.repo, "machine":"testserver"});
        let out = Command::new(env!("CARGO_BIN_EXE_herdr-annotator"))
            .args(["request", &value.to_string()])
            .env("HERDR_ANNOTATOR_HC", &self.program)
            .env("HERDR_ANNOTATOR_STATE_DIR", &self.fixture.state)
            .env("FAKE_CWD", &self.fixture.repo)
            .env("FAKE_LOG", &self.log)
            .env("FAKE_STATUS", status)
            .env("FAKE_FAIL_SEND", if fail_send { "1" } else { "0" })
            .output()
            .unwrap();
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn added(&self) -> Value {
        let s = self.request(json!({"op":"snapshot"}), "idle", false);
        assert_eq!(s["ok"], true);
        let added = self.request(json!({"op":"add", "revision":s["data"]["snapshot"]["revision"], "path":"file.txt", "selection":null, "body":"Human note"}), "idle", false);
        assert_eq!(added["ok"], true);
        added
    }
    fn sends(&self) -> usize {
        fs::read_to_string(&self.log)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str::<Value>(s).unwrap())
            .filter(|v| v[0] == "send")
            .count()
    }
}

#[cfg(unix)]
#[test]
fn remote_reads_and_confirmed_send_use_only_core_with_idle_guard() {
    let core = FakeCore::new();
    let s = core.added();
    let sent = core.request(json!({"op":"send", "target":"testserver/t1", "revision":s["data"]["snapshot"]["revision"]}), "idle", false);
    assert_eq!(sent["ok"], true, "{sent}");
    assert_eq!(sent["data"]["comments"][0]["delivery"], "sent");
    let again = core.request(json!({"op":"send", "target":"testserver/t1", "revision":s["data"]["snapshot"]["revision"]}), "idle", false);
    assert_eq!(again["ok"], false);
    assert_eq!(core.sends(), 1);
    let calls = fs::read_to_string(&core.log).unwrap();
    assert!(calls.contains("--if"));
    assert!(!calls.contains("--force"));
}

#[cfg(unix)]
#[test]
fn failed_send_is_uncertain_and_never_automatically_retried() {
    let core = FakeCore::new();
    let s = core.added();
    let failed = core.request(json!({"op":"send", "target":"testserver/t1", "revision":s["data"]["snapshot"]["revision"]}), "idle", true);
    assert_eq!(failed["ok"], false);
    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .contains("Delivery not confirmed")
    );
    let loaded = core.request(json!({"op":"snapshot"}), "idle", false);
    assert_eq!(loaded["data"]["comments"][0]["delivery"], "uncertain");
    let again = core.request(json!({"op":"send", "target":"testserver/t1", "revision":s["data"]["snapshot"]["revision"]}), "idle", false);
    assert_eq!(again["ok"], false);
    assert_eq!(core.sends(), 1);
}

#[cfg(unix)]
#[test]
fn busy_or_blocked_agents_never_receive_review_text() {
    let core = FakeCore::new();
    let s = core.added();
    for status in ["working", "blocked", "error"] {
        let rejected = core.request(json!({"op":"send", "target":"testserver/t1", "revision":s["data"]["snapshot"]["revision"]}), status, false);
        assert_eq!(rejected["ok"], false);
    }
    assert_eq!(core.sends(), 0);
}

#[test]
fn branch_scope_combines_committed_staged_unstaged_and_untracked_changes() {
    let f = Fixture::new(true);
    git(&f.repo, &["switch", "-c", "123-feature"]);
    fs::write(f.repo.join("file.txt"), "committed feature\n").unwrap();
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-m", "feature"]);
    fs::write(
        f.repo.join("file.txt"),
        "committed feature\nunstaged change\n",
    )
    .unwrap();
    fs::write(f.repo.join("staged.txt"), "staged addition\n").unwrap();
    git(&f.repo, &["add", "staged.txt"]);
    fs::write(f.repo.join("new.txt"), "untracked addition\n").unwrap();
    let index = fs::read(f.repo.join(".git/index")).unwrap();
    let branch = f.branch_request(json!({"op":"snapshot"}), "main").unwrap();
    let files = branch["snapshot"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 3);
    assert!(
        serde_json::to_string(files)
            .unwrap()
            .contains("committed feature")
    );
    assert!(
        serde_json::to_string(files)
            .unwrap()
            .contains("unstaged change")
    );
    assert!(files.iter().any(|file| file["path"] == "staged.txt"));
    assert!(files.iter().any(|file| file["path"] == "new.txt"));
    let file = files.iter().find(|f| f["path"] == "file.txt").unwrap();
    assert!(
        file["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "delete" && r["text"] == "one")
    );
    assert_eq!(fs::read(f.repo.join(".git/index")).unwrap(), index);
    let local = f.snapshot();
    assert!(
        local["snapshot"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == "file.txt")
            .unwrap()["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "context" && r["text"] == "committed feature")
    );
}

#[test]
fn branch_scope_uses_merge_base_not_the_newer_base_tip() {
    let f = Fixture::new(true);
    git(&f.repo, &["switch", "-c", "feature"]);
    fs::write(f.repo.join("feature.txt"), "feature\n").unwrap();
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-m", "feature"]);
    git(&f.repo, &["switch", "main"]);
    fs::write(f.repo.join("base-only.txt"), "new base work\n").unwrap();
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-m", "base advances"]);
    git(&f.repo, &["switch", "feature"]);
    let s = f.branch_request(json!({"op":"snapshot"}), "main").unwrap();
    assert_eq!(s["snapshot"]["files"].as_array().unwrap().len(), 1);
    assert_eq!(s["snapshot"]["files"][0]["path"], "feature.txt");
    assert!(s["snapshot"]["merge_base"].as_str().is_some());
}

#[test]
fn branch_and_working_tree_annotations_are_isolated_and_legacy_store_stays_readable() {
    let f = Fixture::new(true);
    f.change();
    f.add(Value::Null);
    let branch = f.branch_request(json!({"op":"snapshot"}), "main").unwrap();
    assert_eq!(branch["comments"].as_array().unwrap().len(), 0);
    f.branch_request(json!({"op":"add","revision":branch["snapshot"]["revision"],"path":"file.txt","selection":null,"body":"Branch note"}), "main").unwrap();
    assert_eq!(f.snapshot()["comments"].as_array().unwrap().len(), 1);
    assert_eq!(
        f.branch_request(json!({"op":"snapshot"}), "main").unwrap()["comments"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        f.branch_request(json!({"op":"snapshot"}), "not-fetched")
            .unwrap_err()
            .contains("not available locally")
    );
}

#[test]
fn comparison_rejects_stale_revision_from_a_different_scope() {
    let f = Fixture::new(true);
    f.change();
    let local = f.snapshot();
    assert!(f.branch_request(json!({"op":"add", "revision":local["snapshot"]["revision"], "path":"file.txt", "selection":null, "body":"wrong scope"}), "main").unwrap_err().contains("Diff changed"));
}

#[test]
fn large_reviews_list_every_file_and_load_a_file_beyond_the_old_cutoff() {
    let f = Fixture::new(true);
    for n in 0..250 {
        fs::write(
            f.repo.join(format!("item-{n:03}.txt")),
            format!("file {n}\n"),
        )
        .unwrap();
    }
    let first = f.snapshot();
    let files = first["snapshot"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 250);
    assert_eq!(files.iter().filter(|f| f["loaded"] == true).count(), 1);
    assert_eq!(first["snapshot"]["limited"], false);
    let view = json!({"file":"item-249.txt","context":3});
    let late = f.request(json!({"op":"snapshot","view":view})).unwrap();
    let file = late["snapshot"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "item-249.txt")
        .unwrap();
    assert_eq!(file["loaded"], true);
    assert!(file.to_string().contains("file 249"));
    f.request(
        json!({"op":"add","view":view,"revision":late["snapshot"]["revision"],"path":"item-249.txt",
        "selection":{"side":"new","start":1,"end":1},"body":"Late file note"}),
    )
    .unwrap();
    let first = f.snapshot();
    assert_eq!(first["comments"][0]["stale"], false);
    assert_eq!(
        first["snapshot"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["loaded"] == true)
            .count(),
        2
    );
    let exported = f
        .request(json!({"op":"export","revision":first["snapshot"]["revision"]}))
        .unwrap();
    assert!(
        exported["text"]
            .as_str()
            .unwrap()
            .contains("Late file note")
    );
}

#[test]
fn unchanged_context_expands_and_its_notes_survive_collapsing() {
    let f = Fixture::new(true);
    let original = (1..=120)
        .map(|n| format!("source line {n}\n"))
        .collect::<String>();
    fs::write(f.repo.join("file.txt"), &original).unwrap();
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-m", "long baseline"]);
    fs::write(
        f.repo.join("file.txt"),
        original.replace("source line 60\n", "changed line 60\n"),
    )
    .unwrap();
    let small = f.snapshot();
    assert!(
        !small["snapshot"]["files"][0]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["new"] == 40)
    );
    let view = json!({"file":"file.txt","context":23});
    let expanded = f.request(json!({"op":"snapshot","view":view})).unwrap();
    assert!(
        expanded["snapshot"]["files"][0]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["new"] == 40 && r["kind"] == "context")
    );
    assert!(f.request(json!({"op":"add","view":view,"revision":small["snapshot"]["revision"],"path":"file.txt","body":"wrong view","selection":null})).is_err());
    f.request(json!({"op":"add","view":view,"revision":expanded["snapshot"]["revision"],"path":"file.txt","body":"Unchanged context note",
        "selection":{"side":"new","start":40,"end":40}})).unwrap();
    let collapsed = f.snapshot();
    assert_eq!(collapsed["comments"][0]["stale"], false);
    assert!(
        !collapsed["snapshot"]["files"][0]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["new"] == 40)
    );
    let exported = f
        .request(json!({"op":"export","revision":collapsed["snapshot"]["revision"]}))
        .unwrap();
    assert!(
        exported["text"]
            .as_str()
            .unwrap()
            .contains("Unchanged context note")
    );
    let full = f
        .request(json!({"op":"snapshot","view":{"file":"file.txt","context":1000000}}))
        .unwrap();
    assert!(
        full["snapshot"]["files"][0]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["new"] == 120)
    );
    assert!(
        f.request(json!({"op":"snapshot","view":{"context":1000001}}))
            .is_err()
    );
    fs::write(
        f.repo.join("file.txt"),
        original
            .replace("source line 60\n", "changed line 60\n")
            .replace("source line 40\n", "edited context 40\n"),
    )
    .unwrap();
    assert_eq!(f.snapshot()["comments"][0]["stale"], true);
}

#[test]
fn read_markers_persist_without_resolving_or_excluding_feedback() {
    let f = Fixture::new(true);
    f.change();
    let added = f.add(Value::Null);
    let id = &added["comments"][0]["id"];
    assert_eq!(added["comments"][0]["read"], false);
    let read = f.request(json!({"op":"read","id":id,"read":true})).unwrap();
    assert_eq!(read["comments"][0]["read"], true);
    assert_eq!(read["comments"][0]["resolved"], false);
    assert_eq!(read["comments"][0]["delivery"], Value::Null);
    let persisted = f.snapshot();
    assert_eq!(persisted["comments"][0]["read"], true);
    assert_eq!(
        persisted["snapshot"]["revision"],
        added["snapshot"]["revision"]
    );
    assert!(
        f.request(json!({"op":"export","revision":persisted["snapshot"]["revision"]}))
            .is_ok()
    );
    let unread = f
        .request(json!({"op":"read","id":id,"read":false}))
        .unwrap();
    assert_eq!(unread["comments"][0]["read"], false);
    assert!(
        f.request(json!({"op":"read","id":"unknown","read":true}))
            .is_err()
    );
}
