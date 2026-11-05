//! Desktop apps do not inherit interactive-shell tool paths. Prefer installer
//! discoveries, then PATH and known user install locations. Never source a shell.
use serde_json::Value;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
    process::Command,
};

fn executable(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(windows)]
    {
        windows_native_executable(path)
    }
}

// Batch/npm launchers need a separate invocation adapter. Never silently run
// them through a shell with review text in its command line.
#[cfg(windows)]
fn windows_native_executable(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("exe") || extension.eq_ignore_ascii_case("com")
        })
}

fn installed() -> BTreeMap<String, PathBuf> {
    crate::platform::home()
        .and_then(|home| std::fs::read(home.join(".local/share/herdr-annotator/install.json")).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value.get("tools").cloned())
        .and_then(|tools| serde_json::from_value(tools).ok())
        .unwrap_or_default()
}

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<_> = env::var_os("PATH")
        .map(|path| {
            env::split_paths(&path)
                .filter(|p| p.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    if let Some(home) = crate::platform::home() {
        for relative in [
            ".opencode/bin",
            ".local/bin",
            "Library/pnpm/bin",
            ".local/share/pnpm",
            ".local/share/fnm/aliases/default/bin",
            ".fnm/aliases/default/bin",
        ] {
            dirs.push(home.join(relative));
        }
    }
    #[cfg(unix)]
    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    #[cfg(windows)]
    {
        for (variable, relative) in [
            ("LOCALAPPDATA", "Microsoft/WinGet/Links"),
            ("LOCALAPPDATA", "Programs/nodejs"),
            ("APPDATA", "npm"),
            ("ProgramFiles", "nodejs"),
            ("ProgramFiles", "Git/cmd"),
            ("ProgramFiles", "GitHub CLI"),
            ("LOCALAPPDATA", "Programs/Git/cmd"),
            ("LOCALAPPDATA", "pnpm"),
        ] {
            if let Some(base) = env::var_os(variable).filter(|value| !value.is_empty()) {
                dirs.push(PathBuf::from(base).join(relative));
            }
        }
        if let Some(home) = crate::platform::home() {
            dirs.extend([
                home.join(".cargo/bin"),
                home.join(".bun/bin"),
                home.join("scoop/shims"),
            ]);
        }
    }
    dirs
}

#[cfg(windows)]
fn candidates(dir: &Path, name: &str) -> Vec<PathBuf> {
    if Path::new(name).extension().is_none() {
        vec![
            dir.join(format!("{name}.exe")),
            dir.join(format!("{name}.com")),
        ]
    } else {
        vec![dir.join(name)]
    }
}

#[cfg(unix)]
fn candidates(dir: &Path, name: &str) -> Vec<PathBuf> {
    vec![dir.join(name)]
}

pub(crate) fn tool(name: &str, override_name: &str) -> PathBuf {
    resolve(
        name,
        env::var_os(override_name),
        &installed(),
        &search_dirs(),
    )
}

fn resolve(
    name: &str,
    override_path: Option<std::ffi::OsString>,
    receipt: &BTreeMap<String, PathBuf>,
    dirs: &[PathBuf],
) -> PathBuf {
    if let Some(path) = override_path {
        return path.into();
    }
    if let Some(path) = receipt.get(name).filter(|path| usable(path, name)) {
        return path.clone();
    }
    dirs.iter()
        .flat_map(|dir| {
            let paths = candidates(dir, name);
            #[cfg(windows)]
            let paths = paths
                .into_iter()
                .chain([dir.join(format!("{name}.cmd"))])
                .collect::<Vec<_>>();
            paths
        })
        .find(|path| usable(path, name))
        .unwrap_or_else(|| name.into())
}

fn usable(path: &Path, _name: &str) -> bool {
    if executable(path) {
        return true;
    }
    #[cfg(windows)]
    {
        npm_entry(path, _name).is_some()
    }
    #[cfg(unix)]
    {
        false
    }
}

/// Build an argv-based invocation, never a shell command. Only the two known
/// npm runners may resolve a .cmd shim to its package's declared Node entry.
pub(crate) fn command(name: &str, override_name: &str) -> crate::Result<Command> {
    #[cfg(unix)]
    let command = Command::new(tool(name, override_name));
    #[cfg(windows)]
    let command = {
        let path = tool(name, override_name);
        if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
        {
            let entry = npm_entry(&path, name).ok_or_else(|| format!("Unsupported {name} shell launcher; configure a native executable or a supported npm installation"))?;
            // npm prefers node.exe beside its shim, then the discovered runtime.
            let adjacent = path.parent().unwrap_or(Path::new("")).join("node.exe");
            let mut command =
                if env::var_os("HERDR_ANNOTATOR_NODE").is_none() && executable(&adjacent) {
                    Command::new(adjacent)
                } else {
                    crate::executables::command("node", "HERDR_ANNOTATOR_NODE")?
                };
            command.arg(entry);
            command
        } else {
            Command::new(path)
        }
    };
    let mut command = command;
    command.env("PATH", runtime_path());
    Ok(command)
}

#[cfg(any(windows, test))]
fn npm_entry(shim: &Path, name: &str) -> Option<PathBuf> {
    let package = match name {
        "pi" => "@mariozechner/pi-coding-agent",
        "opencode" => "opencode-ai",
        _ => return None,
    };
    if !shim.is_absolute()
        || !shim.is_file()
        || !shim.extension()?.to_str()?.eq_ignore_ascii_case("cmd")
    {
        return None;
    }
    let root = shim
        .parent()?
        .join("node_modules")
        .join(package)
        .canonicalize()
        .ok()?;
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("package.json")).ok()?).ok()?;
    if manifest.get("name")?.as_str()? != package {
        return None;
    }
    let bin = manifest.get("bin")?;
    let relative = bin.as_str().or_else(|| bin.get(name)?.as_str())?;
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative.components().any(|c| {
            !matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return None;
    }
    let entry = root.join(relative).canonicalize().ok()?;
    (entry.starts_with(&root) && entry.is_file()).then_some(entry)
}

pub(crate) fn runtime_path() -> std::ffi::OsString {
    let mut dirs: Vec<_> = installed()
        .iter()
        .filter(|(name, path)| usable(path, name))
        .filter_map(|(_, path)| path.parent().map(Path::to_path_buf))
        .collect();
    dirs.extend(search_dirs());
    // Pass this explicitly through hc exec: core has its own process environment.
    env::join_paths(dirs).unwrap_or_else(|_| env::var_os("PATH").unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn native_windows_extensions_do_not_accept_shell_launchers() {
        for name in ["tool.exe", "tool.EXE", "tool.com"] {
            assert!(windows_native_executable(Path::new(name)));
        }
        for name in ["tool", "tool.cmd", "tool.bat", "tool.ps1", "tool.exe.txt"] {
            assert!(!windows_native_executable(Path::new(name)));
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_candidates_preserve_explicit_extensions() {
        let dir = Path::new("tools");
        assert_eq!(
            candidates(dir, "hc"),
            vec![dir.join("hc.exe"), dir.join("hc.com")]
        );
        assert_eq!(candidates(dir, "hc.exe"), vec![dir.join("hc.exe")]);
    }

    #[cfg(unix)]
    #[test]
    fn unix_candidates_keep_the_exact_name() {
        let dir = Path::new("tools");
        assert_eq!(candidates(dir, "hc"), vec![dir.join("hc")]);
    }

    #[test]
    fn discovery_requires_absolute_existing_files() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!executable(Path::new("relative.exe")));
        assert!(!executable(&temp.path().join("missing.exe")));
        assert!(!executable(temp.path()));
        let file = temp.path().join("tool.exe");
        std::fs::write(&file, "fixture").unwrap();
        #[cfg(unix)]
        {
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(!executable(&file));
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert!(executable(&file));
    }

    #[test]
    fn discovery_prefers_overrides_then_valid_receipts_then_search_paths() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first tools (中文)");
        let second = temp.path().join("second tools & 'quotes'");
        for dir in [&first, &second] {
            std::fs::create_dir(dir).unwrap();
            #[cfg(windows)]
            let path = dir.join("git.exe");
            #[cfg(unix)]
            let path = dir.join("git");
            std::fs::write(&path, "fixture").unwrap();
            #[cfg(unix)]
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let first_tool = candidates(&first, "git")[0].clone();
        let second_tool = candidates(&second, "git")[0].clone();
        let dirs = vec![first, second];
        let mut receipt = BTreeMap::from([("git".into(), second_tool.clone())]);
        assert_eq!(resolve("git", None, &receipt, &dirs), second_tool);
        let explicit = std::ffi::OsString::from("explicit-override");
        assert_eq!(
            resolve("git", Some(explicit.clone()), &receipt, &dirs),
            PathBuf::from(explicit)
        );
        receipt.insert("git".into(), PathBuf::from("relative/path"));
        assert_eq!(resolve("git", None, &receipt, &dirs), first_tool);
        receipt.insert("git".into(), temp.path().join("missing.exe"));
        assert_eq!(resolve("git", None, &receipt, &dirs), first_tool);
        assert_eq!(resolve("git", None, &receipt, &[]), PathBuf::from("git"));
    }

    #[test]
    fn npm_entries_use_manifests_not_shell_launcher_contents() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("tools (test) ' Unicode 中文 & %PATH%");
        let root = dir.join("node_modules/@mariozechner/pi-coding-agent");
        std::fs::create_dir_all(root.join("dist")).unwrap();
        let shim = dir.join("pi.cmd");
        std::fs::write(&shim, "do not execute or parse this shell text").unwrap();
        let entry = root.join("dist/cli.js");
        std::fs::write(&entry, "fixture").unwrap();
        let manifest = root.join("package.json");
        std::fs::write(
            &manifest,
            r#"{"name":"@mariozechner/pi-coding-agent","bin":{"pi":"dist/cli.js"}}"#,
        )
        .unwrap();
        assert_eq!(npm_entry(&shim, "pi"), Some(entry.canonicalize().unwrap()));
        assert_eq!(npm_entry(&shim, "git"), None);
        for bin in ["../outside.js", "/absolute.js", "missing.js"] {
            std::fs::write(
                &manifest,
                serde_json::json!({"name":"@mariozechner/pi-coding-agent", "bin":{"pi":bin}})
                    .to_string(),
            )
            .unwrap();
            assert_eq!(npm_entry(&shim, "pi"), None);
        }
        std::fs::write(
            &manifest,
            r#"{"name":"another-package","bin":"dist/cli.js"}"#,
        )
        .unwrap();
        assert_eq!(npm_entry(&shim, "pi"), None);
    }
}
