#![cfg(unix)]
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
use tempfile::TempDir;

struct Fixture {
    _dir: TempDir,
    repo: std::path::PathBuf,
    state: std::path::PathBuf,
    program: std::path::PathBuf,
    data: std::path::PathBuf,
    log: std::path::PathBuf,
}

fn git(repo: &std::path::Path, args: &[&str]) {
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
fn page(nodes: Value, next: Option<&str>) -> Value {
    json!({"nodes":nodes,"pageInfo":{"hasNextPage":next.is_some(),"endCursor":next}})
}
fn comment(id: &str, body: &str) -> Value {
    json!({"id":id,"body":body,"url":format!("https://github.com/owner/repo/pull/1#discussion_{id}"),
        "updatedAt":body,"author":{"login":"reviewer"},"diffHunk":"@@ -1 +1 @@\n-old\n+new"})
}
fn thread() -> Value {
    json!({"id":"thread1","path":"file.txt","line":2,"startLine":null,"originalLine":2,"originalStartLine":null,
        "diffSide":"RIGHT","isResolved":false,"isOutdated":false,"comments":page(json!([comment("c1","Please handle the empty input")]),None)})
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "--initial-branch=feature"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@example.invalid"]);
        git(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/owner/repo.git",
            ],
        );
        fs::write(repo.join("file.txt"), "old\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "base"]);
        fs::write(repo.join("file.txt"), "new\n").unwrap();
        let state = dir.path().join("state");
        let program = dir.path().join("fake");
        let data = dir.path().join("data.json");
        let log = dir.path().join("calls.jsonl");
        fs::write(&program,r#"#!/usr/bin/env python3
import sys,os,json
a=sys.argv[1:]
with open(os.environ['FAKE_LOG'],'a') as f:f.write(json.dumps(a)+'\n')
d=json.load(open(os.environ['FAKE_DATA']))
if a[0]=='api':
    assert a[1:3]==['--hostname','github.com']
    if d.get('fail'):sys.exit(1)
    if 'graphql' not in a: print(json.dumps(d['prs']));sys.exit(0)
    fields={s.split('=',1)[0]:s.split('=',1)[1] for s in a if '=' in s}
    if 'reviewThreads' in fields['query']:
        result=d['pages'].get(fields.get('cursor','first'))
        print(json.dumps({'data':{'repository':{'pullRequest':{'state':d.get('pr_state','OPEN'),'reviewThreads':result}}}}));sys.exit(0)
    print(json.dumps({'data':{'node':{'comments':d['replies'][fields['cursor']]}}}));sys.exit(0)
if a[0]=='get':
    print(json.dumps({'state':'live','status':{'state':'idle','app':'opencode'},'foreground':{'cwd':os.environ['FAKE_CWD']}}));sys.exit(0)
if a[0]=='send':
    if d.get('send_fail'):sys.exit(2)
    print(json.dumps({'turn':1,'started':True}));sys.exit(0)
sys.exit(2)
"#).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(&data,json!({"prs":[{"number":1,"html_url":"https://github.com/owner/repo/pull/1","head":{"ref":"feature","repo":{"full_name":"owner/repo"}},"base":{"ref":"main"}}],
            "pages":{"first":page(json!([thread()]),None)},"replies":{}}).to_string()).unwrap();
        Self {
            _dir: dir,
            repo,
            state,
            program,
            data,
            log,
        }
    }
    fn request(&self, mut value: Value) -> Value {
        value["scope"] = json!({"repo":self.repo,"machine":null});
        let out = Command::new(env!("CARGO_BIN_EXE_herdr-annotator"))
            .args(["request", &value.to_string()])
            .env("HERDR_ANNOTATOR_STATE_DIR", &self.state)
            .env("HERDR_ANNOTATOR_GH", &self.program)
            .env("HERDR_ANNOTATOR_HC", &self.program)
            .env("FAKE_DATA", &self.data)
            .env("FAKE_LOG", &self.log)
            .env("FAKE_CWD", &self.repo)
            .output()
            .unwrap();
        serde_json::from_slice(&out.stdout).unwrap()
    }
    fn update(&self, f: impl FnOnce(&mut Value)) {
        let mut data: Value = serde_json::from_slice(&fs::read(&self.data).unwrap()).unwrap();
        f(&mut data);
        fs::write(&self.data, data.to_string()).unwrap();
    }
    fn calls(&self, op: &str) -> Vec<Value> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|v| v[0] == op)
            .collect()
    }
    fn refs(&self) -> Value {
        let gh = self.request(json!({"op":"github","force":true}));
        assert_eq!(gh["ok"], true, "{gh}");
        let t = &gh["data"]["github"]["threads"][0];
        json!([{"id":t["id"],"fingerprint":t["fingerprint"]}])
    }
    fn revision(&self) -> Value {
        let s = self.request(json!({"op":"snapshot"}));
        assert_eq!(s["ok"], true, "{s}");
        s["data"]["snapshot"]["revision"].clone()
    }
}

#[test]
fn github_reads_are_cached_for_five_minutes_and_refresh_is_immediate() {
    let f = Fixture::new();
    let first = f.request(json!({"op":"github"}));
    assert_eq!(first["data"]["github"]["pull_request"]["base"], "main");
    assert_eq!(f.calls("api").len(), 2);
    f.request(json!({"op":"github"}));
    assert_eq!(f.calls("api").len(), 2);
    f.request(json!({"op":"github","force":true}));
    assert_eq!(f.calls("api").len(), 4);
    assert_eq!(f.calls("send").len(), 0);
}

#[test]
fn pagination_collects_threads_and_all_replies() {
    let f = Fixture::new();
    f.update(|d| {
        let mut t = thread();
        t["comments"] = page(json!([comment("c1", "First")]), Some("reply2"));
        let mut t2 = thread();
        t2["id"] = json!("thread2");
        d["pages"]["first"] = page(json!([t]), Some("threadpage2"));
        d["pages"]["threadpage2"] = page(json!([t2]), None);
        d["replies"]["reply2"] = page(json!([comment("c2", "Second reply")]), None);
    });
    let gh = f.request(json!({"op":"github"}));
    assert_eq!(gh["data"]["github"]["threads"].as_array().unwrap().len(), 2);
    assert_eq!(
        gh["data"]["github"]["threads"][0]["comments"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn fix_thread_sends_context_but_not_unrelated_local_comments() {
    let f = Fixture::new();
    let revision = f.revision();
    assert_eq!(f.request(json!({"op":"add","revision":revision,"path":"file.txt","selection":null,"body":"UNRELATED-LOCAL-NOTE"}))["ok"],true);
    let refs = f.refs();
    let sent = f.request(
        json!({"op":"send","revision":revision,"target":"t7","threads":refs,"include_local":false}),
    );
    assert_eq!(sent["ok"], true, "{sent}");
    assert_eq!(sent["data"]["comments"][0]["delivery"], Value::Null);
    assert_eq!(sent["data"]["github"]["threads"][0]["delivery"], "sent");
    let calls = f.calls("send");
    let prompt = calls[0]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .as_str()
        .unwrap();
    assert!(prompt.contains("Please handle the empty input"));
    assert!(prompt.contains("diff context"));
    assert!(!prompt.contains("UNRELATED-LOCAL-NOTE"));
    let duplicate = f.request(
        json!({"op":"send","revision":revision,"target":"t7","threads":refs,"include_local":false}),
    );
    assert_eq!(duplicate["ok"], false);
    assert_eq!(f.calls("send").len(), 1);
}

#[test]
fn batch_combines_local_notes_with_threads_and_new_replies_require_reselection() {
    let f = Fixture::new();
    let revision = f.revision();
    f.request(json!({"op":"add","revision":revision,"path":"file.txt","selection":null,"body":"Local batch note"}));
    let refs = f.refs();
    f.update(|d| {
        d["pages"]["first"]["nodes"][0]["comments"]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(comment("c2", "New reply"))
    });
    let stale = f.request(json!({"op":"send","revision":revision,"target":"t7","threads":refs}));
    assert_eq!(stale["ok"], false);
    assert_eq!(f.calls("send").len(), 0);
    let current = f.refs();
    let sent = f.request(json!({"op":"send","revision":revision,"target":"t7","threads":current}));
    assert_eq!(sent["ok"], true, "{sent}");
    assert_eq!(sent["data"]["comments"][0]["delivery"], "sent");
    let calls = f.calls("send");
    let prompt = calls[0]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .as_str()
        .unwrap();
    assert!(prompt.contains("Local batch note") && prompt.contains("New reply"));
    f.update(|d| {
        d["pages"]["first"]["nodes"][0]["comments"]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(comment("c3", "Another reply"))
    });
    let gh = f.request(json!({"op":"github","force":true}));
    assert_eq!(gh["data"]["github"]["threads"][0]["has_updates"], true);
}

#[test]
fn refresh_failures_preserve_threads_but_prevent_sending_and_resolved_threads_are_refused() {
    let f = Fixture::new();
    let refs = f.refs();
    let revision = f.revision();
    f.update(|d| d["fail"] = json!(true));
    let gh = f.request(json!({"op":"github","force":true}));
    assert!(gh["data"]["github"]["error"].is_string());
    assert_eq!(gh["data"]["github"]["threads"].as_array().unwrap().len(), 1);
    assert_eq!(
        f.request(json!({"op":"send","revision":revision,"target":"t7","threads":refs}))["ok"],
        false
    );
    f.update(|d| {
        d["fail"] = json!(false);
        d["pages"]["first"]["nodes"][0]["isResolved"] = json!(true);
    });
    let current = f.refs();
    assert_eq!(
        f.request(json!({"op":"send","revision":revision,"target":"t7","threads":current}))["ok"],
        false
    );
    assert_eq!(f.calls("send").len(), 0);
}

#[test]
fn uncertain_thread_delivery_is_persisted_and_not_retried() {
    let f = Fixture::new();
    let refs = f.refs();
    let revision = f.revision();
    f.update(|d| d["send_fail"] = json!(true));
    assert_eq!(
        f.request(json!({"op":"send","revision":revision,"target":"t7","threads":refs}))["ok"],
        false
    );
    let gh = f.request(json!({"op":"github"}));
    assert_eq!(gh["data"]["github"]["threads"][0]["delivery"], "uncertain");
    f.update(|d| d["send_fail"] = json!(false));
    assert_eq!(
        f.request(json!({"op":"send","revision":revision,"target":"t7","threads":refs}))["ok"],
        false
    );
    assert_eq!(f.calls("send").len(), 1);
}

#[test]
fn missing_pr_and_non_github_remotes_are_accounted_for_without_sending() {
    let f = Fixture::new();
    f.update(|d| d["prs"] = json!([]));
    let result = f.request(json!({"op":"github"}));
    assert_eq!(result["data"]["github"]["pull_request"], Value::Null);
    git(
        &f.repo,
        &[
            "remote",
            "set-url",
            "origin",
            "https://evil.invalid/owner/repo",
        ],
    );
    let calls = f.calls("api").len();
    let result = f.request(json!({"op":"github","force":true}));
    assert!(result["data"]["github"]["error"].is_string());
    assert_eq!(f.calls("api").len(), calls);
}

#[test]
fn github_read_markers_are_local_and_new_replies_become_unread() {
    let f = Fixture::new();
    let refs = f.refs();
    let thread = &refs[0];
    let calls = f.calls("api").len();
    let marked = f.request(json!({"op":"github_read","thread":thread,"read":true}));
    assert_eq!(marked["ok"], true, "{marked}");
    assert_eq!(marked["data"]["github"]["threads"][0]["read"], true);
    assert_eq!(marked["data"]["github"]["threads"][0]["resolved"], false);
    assert_eq!(
        f.calls("api").len(),
        calls,
        "read markers must never write to GitHub"
    );
    let persisted = f.request(json!({"op":"github"}));
    assert_eq!(persisted["data"]["github"]["threads"][0]["read"], true);
    f.update(|d| {
        d["pages"]["first"]["nodes"][0]["comments"]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(comment("c2", "New reply"))
    });
    let current = f.refs();
    assert_eq!(
        f.request(json!({"op":"github"}))["data"]["github"]["threads"][0]["read"],
        false
    );
    assert_eq!(
        f.request(json!({"op":"github_read","thread":thread,"read":true}))["ok"],
        false
    );
    assert_eq!(
        f.request(json!({"op":"github_read","thread":current[0],"read":true}))["data"]["github"]["threads"]
            [0]["read"],
        true
    );
    assert_eq!(
        f.request(json!({"op":"github_read","thread":current[0],"read":false}))["data"]["github"]["threads"]
            [0]["read"],
        false
    );
    assert_eq!(f.calls("send").len(), 0);
}
