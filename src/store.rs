use crate::{Comment, Result, Scope, model::hash, new_id};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    scope: Scope,
    comments: Vec<Comment>,
}

pub struct Store {
    pub comments: Vec<Comment>,
    scope: Scope,
    path: PathBuf,
    _lock: File,
}

impl Store {
    pub fn open(dir: &Path, scope: &Scope) -> Result<Self> {
        let key = hash(&serde_json::to_vec(scope).map_err(|e| e.to_string())?);
        let (lock, path) = lock_state(dir, &key)?;
        let comments = match fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > 12 * 1024 * 1024 {
                    return Err("Review store exceeds 12 MiB; not overwritten".into());
                }
                let doc: Document = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("Invalid review store; not overwritten: {e}"))?;
                if doc.version != 1 || doc.scope != *scope || doc.comments.len() > 500 {
                    return Err("Incompatible or mismatched review store; not overwritten".into());
                }
                let ids: std::collections::HashSet<_> =
                    doc.comments.iter().map(|c| &c.id).collect();
                if ids.len() != doc.comments.len() {
                    return Err("Duplicate comment IDs in review store".into());
                }
                doc.comments
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.to_string()),
        };
        Ok(Self {
            comments,
            scope: scope.clone(),
            path,
            _lock: lock,
        })
    }

    pub fn save(&self) -> Result<()> {
        let doc = Document {
            version: 1,
            scope: self.scope.clone(),
            comments: self.comments.clone(),
        };
        let data = serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?;
        write_atomic(&self.path, &data)
    }
}

pub(crate) fn lock_state(dir: &Path, key: &str) -> Result<(File, PathBuf)> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(dir.join(format!("{key}.lock")))
        .map_err(|e| e.to_string())?;
    fs2::FileExt::lock_exclusive(&file).map_err(|e| e.to_string())?;
    Ok((file, dir.join(format!("{key}.json"))))
}

pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!("{}.tmp", new_id()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temp).map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        // Persist the rename as well as file data on supported Unix filesystems.
        // Windows does not support opening directories with File::open. File
        // data is synced above; directory-entry durability is not promised there.
        #[cfg(unix)]
        File::open(path.parent().unwrap_or(Path::new(".")))?.sync_all()?;
        Ok::<(), std::io::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_writes_create_and_replace_without_leaving_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"replacement").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"replacement");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_replacement_preserves_destination_and_removes_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("directory.json");
        fs::create_dir(&path).unwrap();
        assert!(write_atomic(&path, b"not a directory").is_err());
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn state_lock_excludes_other_handles_and_releases_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let (first, _) = lock_state(dir.path(), "review").unwrap();
        let second = OpenOptions::new()
            .read(true)
            .write(true)
            .open(dir.path().join("review.lock"))
            .unwrap();
        assert!(fs2::FileExt::try_lock_exclusive(&second).is_err());
        drop(first);
        fs2::FileExt::try_lock_exclusive(&second).unwrap();
    }
}
