//! Fake generation only. Never calls a model or touches a working chat.
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    repo: PathBuf,
    runner: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir(&repo).unwrap();
        for args in [
            vec!["init", "--initial-branch=main"],
            vec!["config", "user.name", "Test"],
            vec!["config", "user.email", "test@example.invalid"],
        ] {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(&repo)
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        fs::write(repo.join("a.rs"), "fn caller() { implementation(); }\n").unwrap();
        fs::write(repo.join("b.rs"), "fn implementation() {}\n").unwrap();
        let runner = dir.path().join("opencode");
        fs::write(&runner, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$ORDER_LOG\"\nif [ -n \"$ORDER_MUTATE\" ]; then printf 'changed during generation\\n' > \"$ORDER_MUTATE\"; fi\ncat \"$ORDER_RESPONSE\"\nexit \"${ORDER_EXIT:-0}\"\n").unwrap();
        fs::set_permissions(&runner, fs::Permissions::from_mode(0o700)).unwrap();
        Self { dir, repo, runner }
    }

    fn request(&self, value: Value, response: &str, mutate: bool, exit: &str) -> Value {
        let response_path = self.dir.path().join("response");
        fs::write(&response_path, response).unwrap();
        let mut value = value;
        value["scope"] = json!({"repo":self.repo});
        let output = Command::new(env!("CARGO_BIN_EXE_herdr-annotator"))
            .args(["request", &value.to_string()])
            .env("HERDR_ANNOTATOR_STATE_DIR", self.dir.path().join("state"))
            .env("HERDR_ANNOTATOR_OPENCODE", &self.runner)
            .env("HERDR_ANNOTATOR_HC", &self.runner)
            .env("HERDR_ANNOTATOR_PI", "/fixture/pi")
            .env("ORDER_RESPONSE", response_path)
            .env("ORDER_LOG", self.dir.path().join("calls"))
            .env(
                "ORDER_MUTATE",
                if mutate {
                    self.repo.join("b.rs")
                } else {
                    PathBuf::new()
                },
            )
            .env("ORDER_EXIT", exit)
            .output()
            .unwrap();
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn revision(&self) -> String {
        self.request(json!({"op":"snapshot"}), "", false, "0")["data"]["snapshot"]["revision"]
            .as_str()
            .unwrap()
            .to_owned()
    }
}

#[test]
fn one_stateless_generation_returns_order_without_modifying_files() {
    let f = Fixture::new();
    let revision = f.revision();
    let response = json!({"data":{"text":"[\"b.rs\",\"a.rs\"]"}}).to_string();
    let out = f.request(
        json!({"op":"order","revision":revision}),
        &response,
        false,
        "0",
    );
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["data"]["files"], json!(["b.rs", "a.rs"]));
    let log = fs::read_to_string(f.dir.path().join("calls")).unwrap();
    let args: Vec<_> = log.lines().collect();
    assert_eq!(
        &args[..4],
        &["api", "post", "/api/experimental/generate/text", "--data"]
    );
    let body: Value = serde_json::from_str(args[4]).unwrap();
    assert_eq!(body.as_object().unwrap().len(), 1);
    assert!(body["prompt"].as_str().unwrap().contains("implementation"));
    assert_eq!(
        fs::read_to_string(f.repo.join("b.rs")).unwrap(),
        "fn implementation() {}\n"
    );
    assert_eq!(out["data"]["revision"], revision);
}

#[test]
fn changes_in_an_unselected_file_during_generation_are_rejected() {
    let f = Fixture::new();
    let revision = f.revision();
    let response = json!({"data":{"text":"[\"b.rs\",\"a.rs\"]"}}).to_string();
    let out = f.request(
        json!({"op":"order","revision":revision}),
        &response,
        true,
        "0",
    );
    assert_eq!(out["ok"], false);
    assert!(out["error"].as_str().unwrap().contains("Diff changed"));
}

#[test]
fn stale_input_never_starts_generation_and_failures_do_not_echo_private_data() {
    let f = Fixture::new();
    let out = f.request(
        json!({"op":"order","revision":"stale"}),
        "private source",
        false,
        "0",
    );
    assert_eq!(out["ok"], false);
    assert!(!f.dir.path().join("calls").exists());
    let revision = f.revision();
    for (response, exit) in [
        ("private source", "0"),
        ("private source", "1"),
        (r#"{"data":{"text":"[\"a.rs\",\"a.rs\"]"}}"#, "0"),
    ] {
        let out = f.request(
            json!({"op":"order","revision":revision}),
            response,
            false,
            exit,
        );
        assert_eq!(out["ok"], false);
        assert!(!out.to_string().contains("private source"));
    }
}

#[test]
fn pi_runs_once_through_local_core_without_tools_or_session_history() {
    let f = Fixture::new();
    let revision = f.revision();
    let out = f.request(
        json!({"op":"order","revision":revision,"runner":"pi"}),
        "[\"b.rs\",\"a.rs\"]",
        false,
        "0",
    );
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["data"]["files"], json!(["b.rs", "a.rs"]));
    let log = fs::read_to_string(f.dir.path().join("calls")).unwrap();
    let args: Vec<_> = log.lines().collect();
    assert_eq!(&args[..4], &["exec", "--timeout", "110", "/fixture/pi"]);
    for flag in [
        "-p",
        "--no-session",
        "--no-tools",
        "--no-extensions",
        "--no-mcp",
        "--no-skills",
        "--no-prompt-templates",
        "--no-context-files",
        "--no-approve",
        "--system-prompt",
        "--",
    ] {
        assert!(args.contains(&flag), "Missing {flag}");
    }
    for forbidden in [
        "--on",
        "run",
        "send",
        "--continue",
        "--session",
        "--auto",
        "--approve",
    ] {
        assert!(!args.contains(&forbidden), "Unexpected {forbidden}");
    }
    assert!(log.contains("implementation"));
}

#[test]
fn pi_failures_and_stale_results_do_not_apply_or_leak_output() {
    let f = Fixture::new();
    let revision = f.revision();
    for (text, exit) in [
        ("private source", "1"),
        ("private source", "0"),
        ("[\"a.rs\",\"a.rs\"]", "0"),
    ] {
        let out = f.request(
            json!({"op":"order","revision":revision,"runner":"pi"}),
            text,
            false,
            exit,
        );
        assert_eq!(out["ok"], false, "{out}");
        assert!(!out.to_string().contains("private source"));
    }
    let out = f.request(
        json!({"op":"order","revision":revision,"runner":"pi"}),
        "[\"b.rs\",\"a.rs\"]",
        true,
        "0",
    );
    assert_eq!(out["ok"], false);
    assert!(out["error"].as_str().unwrap().contains("Diff changed"));
}
