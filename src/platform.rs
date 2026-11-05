//! Resolve platform paths without changing the process environment. Installed
//! receipts stay under the installer's existing home-relative directory.
use std::{ffi::OsString, path::PathBuf};

fn value(get: &impl Fn(&'static str) -> Option<OsString>, name: &'static str) -> Option<OsString> {
    get(name).filter(|value| !value.is_empty())
}

#[cfg(windows)]
fn home_from(get: &impl Fn(&'static str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(profile) = value(get, "USERPROFILE") {
        return Some(profile.into());
    }
    if let (Some(mut drive), Some(path)) = (value(get, "HOMEDRIVE"), value(get, "HOMEPATH")) {
        drive.push(path);
        return Some(drive.into());
    }
    value(get, "HOME").map(PathBuf::from)
}

#[cfg(unix)]
fn home_from(get: &impl Fn(&'static str) -> Option<OsString>) -> Option<PathBuf> {
    value(get, "HOME").map(PathBuf::from)
}

pub(crate) fn home() -> Option<PathBuf> {
    home_from(&std::env::var_os)
}

#[cfg(windows)]
fn default_state_dir_from(
    get: &impl Fn(&'static str) -> Option<OsString>,
) -> crate::Result<PathBuf> {
    let local = value(get, "LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| home_from(get).map(|home| home.join("AppData/Local")));
    local
        .map(|local| local.join("herdr-annotator"))
        .ok_or("Windows profile is unset; set HERDR_ANNOTATOR_STATE_DIR".into())
}

#[cfg(unix)]
fn default_state_dir_from(
    get: &impl Fn(&'static str) -> Option<OsString>,
) -> crate::Result<PathBuf> {
    if let Some(path) = value(get, "XDG_STATE_HOME") {
        return Ok(PathBuf::from(path).join("herdr-annotator"));
    }
    home_from(get)
        .map(|home| home.join(".local/state/herdr-annotator"))
        .ok_or("HOME is unset; set HERDR_ANNOTATOR_STATE_DIR".into())
}

fn state_dir_from(get: &impl Fn(&'static str) -> Option<OsString>) -> crate::Result<PathBuf> {
    if let Some(path) = value(get, "HERDR_ANNOTATOR_STATE_DIR") {
        return Ok(path.into());
    }
    default_state_dir_from(get)
}

pub(crate) fn state_dir() -> crate::Result<PathBuf> {
    state_dir_from(&std::env::var_os)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(values: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let values: std::collections::BTreeMap<_, _> = values
            .iter()
            .map(|(key, value)| (key.to_string(), OsString::from(value)))
            .collect();
        move |key| values.get(key).cloned()
    }

    #[cfg(windows)]
    #[test]
    fn windows_home_prefers_native_profile_and_handles_missing_home() {
        let get = environment(&[("USERPROFILE", "profile"), ("HOME", "unix-home")]);
        assert_eq!(home_from(&get), Some(PathBuf::from("profile")));
        let get = environment(&[("HOMEDRIVE", "C:"), ("HOMEPATH", r"\Users\Reviewer")]);
        assert_eq!(home_from(&get), Some(PathBuf::from(r"C:\Users\Reviewer")));
        assert_eq!(home_from(&environment(&[])), None);
    }

    #[cfg(unix)]
    #[test]
    fn unix_home_uses_home_not_windows_profile() {
        let get = environment(&[("USERPROFILE", "profile"), ("HOME", "unix-home")]);
        assert_eq!(home_from(&get), Some(PathBuf::from("unix-home")));
        assert_eq!(home_from(&environment(&[])), None);
    }

    #[test]
    fn state_override_wins_on_both_platforms() {
        let get = environment(&[
            ("HERDR_ANNOTATOR_STATE_DIR", "override"),
            ("LOCALAPPDATA", "local"),
            ("XDG_STATE_HOME", "xdg"),
        ]);
        assert_eq!(state_dir_from(&get).unwrap(), PathBuf::from("override"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_state_uses_local_appdata_or_profile_not_xdg() {
        let get = environment(&[("LOCALAPPDATA", "local"), ("XDG_STATE_HOME", "xdg")]);
        assert_eq!(
            state_dir_from(&get).unwrap(),
            PathBuf::from("local/herdr-annotator")
        );
        let get = environment(&[("USERPROFILE", "profile"), ("LOCALAPPDATA", "")]);
        assert_eq!(
            state_dir_from(&get).unwrap(),
            PathBuf::from("profile/AppData/Local/herdr-annotator")
        );
        assert!(state_dir_from(&environment(&[])).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn unix_state_conventions_are_unchanged() {
        let get = environment(&[("XDG_STATE_HOME", "xdg"), ("HOME", "home")]);
        assert_eq!(
            state_dir_from(&get).unwrap(),
            PathBuf::from("xdg/herdr-annotator")
        );
        let get = environment(&[("HOME", "home"), ("HERDR_ANNOTATOR_STATE_DIR", "")]);
        assert_eq!(
            state_dir_from(&get).unwrap(),
            PathBuf::from("home/.local/state/herdr-annotator")
        );
        assert!(state_dir_from(&environment(&[])).is_err());
    }
}
