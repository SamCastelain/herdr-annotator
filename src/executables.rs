//! Desktop apps do not inherit interactive-shell tool paths. Prefer installer
//! discoveries, then PATH and known user install locations. Never source a shell.
use serde_json::Value;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
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
        ] {
            if let Some(base) = env::var_os(variable).filter(|value| !value.is_empty()) {
                dirs.push(PathBuf::from(base).join(relative));
            }
        }
        if let Some(home) = crate::platform::home() {
            dirs.extend([home.join(".cargo/bin"), home.join(".bun/bin")]);
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
    if let Some(path) = env::var_os(override_name) {
        return path.into();
    }
    if let Some(path) = installed().get(name).filter(|path| executable(path)) {
        return path.clone();
    }
    search_dirs()
        .iter()
        .flat_map(|dir| candidates(dir, name))
        .find(|path| executable(path))
        .unwrap_or_else(|| name.into())
}

pub(crate) fn runtime_path() -> std::ffi::OsString {
    let mut dirs: Vec<_> = installed()
        .values()
        .filter(|path| executable(path))
        .filter_map(|path| path.parent().map(Path::to_path_buf))
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
}
