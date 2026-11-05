//! Read-only discovery. Never select a model from an unrelated/ambiguous chat.
use crate::{OrderingModel, Result, Scope, executables, git, process};
use serde_json::{Value, json};

pub(crate) fn api(path: &str) -> Result<Value> {
    let mut command = executables::command("opencode", "HERDR_ANNOTATOR_OPENCODE")?;
    command.args(["api", "get", path]);
    command.env("PATH", executables::runtime_path());
    let output = process::run(&mut command, 192 * 1024)?;
    if output.code != 0 || output.truncated {
        return Err("Cannot read OpenCode models/sessions. Check local OpenCode V2.".into());
    }
    serde_json::from_str(&output.text).map_err(|_| "Invalid OpenCode discovery response".into())
}

pub(crate) fn validate(model: &OrderingModel) -> Result<()> {
    let valid = |value: &str| {
        !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
    };
    if !valid(&model.id)
        || !valid(&model.provider_id)
        || model.variant.as_deref().is_some_and(|v| !valid(v))
    {
        return Err("Invalid ordering model".into());
    }
    Ok(())
}

fn query(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub(crate) fn source_model(scope: &Scope, target: &str) -> Result<OrderingModel> {
    if scope.machine.is_some() {
        return Err(
            "Remote source model cannot be inferred locally. Choose an ordering model.".into(),
        );
    }
    if !target.starts_with('t')
        || target.len() < 2
        || !target[1..].bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err("Choose a local source terminal ID".into());
    }
    let mut command = executables::command("hc", "HERDR_ANNOTATOR_HC")?;
    command.args(["get", target]);
    let output = process::run(&mut command, 64 * 1024)?;
    if output.code != 0 || output.truncated {
        return Err("Cannot inspect source chat. Choose an ordering model.".into());
    }
    let terminal: Value =
        serde_json::from_str(&output.text).map_err(|_| "Invalid source terminal response")?;
    if terminal["state"] != "live" || terminal["status"]["app"] != "opencode" {
        return Err("Source chat is not OpenCode. Choose an ordering model.".into());
    }
    let title = terminal["title"]
        .as_str()
        .and_then(|s| s.strip_prefix("OC | "))
        .filter(|s| !s.is_empty())
        .ok_or("Source session title is unavailable. Choose an ordering model.")?;
    let directory = terminal["foreground"]["cwd"]
        .as_str()
        .ok_or("Source checkout is unavailable")?;
    let mut target_scope = scope.clone();
    target_scope.repo = directory.to_owned();
    let root = git::required(&target_scope, &["rev-parse", "--show-toplevel"])?;
    let reviewed =
        std::fs::canonicalize(&scope.repo).map_err(|_| "Source checkout is unavailable")?;
    if std::path::Path::new(root.trim_end_matches('\n')) != reviewed {
        return Err("Source chat is in another worktree. Choose an ordering model.".into());
    }
    let path = format!(
        "/api/session?directory={}&parentID=null&archived=false&limit=100&search={}",
        query(directory),
        query(title)
    );
    let mut cursor: Option<String> = None;
    let mut all = Vec::new();
    for _ in 0..5 {
        let page = api(&match &cursor {
            Some(cursor) => format!("{path}&cursor={}", query(cursor)),
            None => path.clone(),
        })?;
        let items = page["data"]
            .as_array()
            .ok_or("Invalid OpenCode session list")?;
        all.extend(items.iter().cloned());
        let next = page["cursor"]["next"].as_str();
        // OpenCode supplies a next cursor even for the final non-empty page.
        if items.is_empty() || next.is_none() {
            return match_source(&json!({"data": all}), directory, title);
        }
        if next == cursor.as_deref() {
            break;
        }
        cursor = next.map(str::to_owned);
    }
    Err("Source session lookup is incomplete. Choose an ordering model.".into())
}

fn match_source(response: &Value, directory: &str, title: &str) -> Result<OrderingModel> {
    let sessions = response["data"]
        .as_array()
        .ok_or("Invalid OpenCode session list")?;
    let matches: Vec<_> = sessions
        .iter()
        .filter(|s| {
            s["parentID"].is_null()
                && s["time"]["archived"].is_null()
                && s["location"]["directory"] == directory
                && s["title"] == title
        })
        .collect();
    if matches.len() != 1 {
        return Err(
            "Cannot identify one source OpenCode session. Choose an ordering model.".into(),
        );
    }
    let model: OrderingModel = serde_json::from_value(matches[0]["model"].clone())
        .map_err(|_| "Source session has no selected model. Choose an ordering model.")?;
    validate(&model)?;
    Ok(model)
}

pub(crate) fn options(scope: &Scope, target: Option<&str>) -> Result<Value> {
    let (source, source_error) = match target {
        Some(target) => match source_model(scope, target) {
            Ok(model) => (Some(model), None),
            Err(error) => (None, Some(error)),
        },
        None => (
            None,
            Some("Source chat unavailable. Choose an ordering model.".to_owned()),
        ),
    };
    let response = api("/api/model")?;
    let models = response["data"]
        .as_array()
        .ok_or("Invalid OpenCode model list")?;
    let choices: Vec<_> = models
        .iter()
        .filter(|m| m["enabled"] == true && m["type"] == "chat")
        .filter_map(|m| {
            let model = OrderingModel {
                id: m["id"].as_str()?.into(),
                provider_id: m["providerID"].as_str()?.into(),
                variant: None,
            };
            validate(&model).ok()?;
            Some(json!({"model": model, "name": m["name"].as_str().unwrap_or(&model.id)}))
        })
        .collect();
    Ok(json!({"source": source, "source_error": source_error, "models": choices}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_never_uses_another_checkout_title_child_or_archived_session() {
        let session = json!({"title":"Source","location":{"directory":"/checkout"},
            "model":{"providerID":"fixture","id":"chat","variant":"low"}});
        assert_eq!(
            match_source(&json!({"data":[session]}), "/checkout", "Source")
                .unwrap()
                .variant
                .as_deref(),
            Some("low")
        );
        for kind in [
            "checkout",
            "title",
            "child",
            "archived",
            "model",
            "duplicate",
        ] {
            let mut changed = session.clone();
            match kind {
                "checkout" => changed["location"]["directory"] = json!("/other"),
                "title" => changed["title"] = json!("Other"),
                "child" => changed["parentID"] = json!("ses_parent"),
                "archived" => changed["time"] = json!({"archived":1}),
                "model" => changed["model"] = Value::Null,
                _ => (),
            }
            let items = if kind == "duplicate" {
                vec![session.clone(), changed]
            } else {
                vec![changed]
            };
            assert!(
                match_source(&json!({"data":items}), "/checkout", "Source").is_err(),
                "{kind}"
            );
        }
    }
}
