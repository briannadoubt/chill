use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use chill_objects::{ImmutableStore, ObjectError};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::LakeFile;

/// Materialized file-cache failure.
#[derive(Debug, Error)]
pub enum FileCacheError {
    /// Root or byte limit is invalid.
    #[error("query file cache configuration is invalid")]
    InvalidConfiguration,
    /// Cache cannot evict enough unreferenced data.
    #[error("query file cache capacity is exhausted")]
    Capacity,
    /// Object fetch failed.
    #[error("query file cache object operation failed: {0}")]
    Object(#[from] ObjectError),
    /// Cache filesystem operation failed.
    #[error("query file cache filesystem operation failed: {0}")]
    Filesystem(#[from] std::io::Error),
    /// Downloaded or existing bytes did not match committed metadata.
    #[error("query file cache object integrity check failed")]
    Integrity,
    /// Internal cache lock was poisoned.
    #[error("query file cache state is unavailable")]
    State,
}

#[derive(Clone)]
struct Entry {
    path: PathBuf,
    size: i64,
    references: usize,
    last_used: Instant,
}

#[derive(Default)]
struct State {
    entries: HashMap<String, Entry>,
    total_bytes: i64,
}

/// Digest-addressed, capacity-bounded local Parquet cache.
#[derive(Clone)]
pub struct FileCache {
    root: PathBuf,
    maximum_bytes: i64,
    objects: ImmutableStore,
    state: Arc<Mutex<State>>,
}

/// One reference-counted set of local immutable files.
pub struct AcquiredFiles {
    /// Local paths in catalog order.
    pub paths: Vec<String>,
    keys: Vec<String>,
    state: Arc<Mutex<State>>,
}

impl AcquiredFiles {
    /// Releases eviction protection for all acquired paths.
    pub async fn release(self) {
        let mut state = self.state.lock().await;
        for key in self.keys {
            if let Some(entry) = state.entries.get_mut(&key) {
                entry.references = entry.references.saturating_sub(1);
                entry.last_used = Instant::now();
            }
        }
    }
}

impl FileCache {
    /// Creates a private cache directory.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty root, a sub-MiB capacity, or a filesystem
    /// setup/canonicalization failure.
    pub fn new(
        root: impl AsRef<Path>,
        maximum_bytes: i64,
        objects: ImmutableStore,
    ) -> Result<Self, FileCacheError> {
        if root.as_ref().as_os_str().is_empty() || maximum_bytes < 1 << 20 {
            return Err(FileCacheError::InvalidConfiguration);
        }
        fs::create_dir_all(root.as_ref())?;
        fs::set_permissions(root.as_ref(), private_directory_permissions())?;
        let root = root.as_ref().canonicalize()?;
        Ok(Self {
            root,
            maximum_bytes,
            objects,
            state: Arc::new(Mutex::new(State::default())),
        })
    }

    /// Returns the canonical `DuckDB`-allowed root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Materializes and references all files in catalog order.
    ///
    /// # Errors
    ///
    /// Returns an error for capacity exhaustion, object failure, filesystem
    /// failure, or a byte-count/digest mismatch.
    pub async fn acquire(&self, files: &[LakeFile]) -> Result<AcquiredFiles, FileCacheError> {
        let mut acquired_keys = Vec::with_capacity(files.len());
        let mut paths = Vec::with_capacity(files.len());
        for file in files {
            match self.acquire_one(file).await {
                Ok((key, path)) => {
                    acquired_keys.push(key);
                    paths.push(path.to_string_lossy().into_owned());
                }
                Err(error) => {
                    self.release_keys(&acquired_keys).await;
                    return Err(error);
                }
            }
        }
        Ok(AcquiredFiles {
            paths,
            keys: acquired_keys,
            state: Arc::clone(&self.state),
        })
    }

    async fn acquire_one(&self, file: &LakeFile) -> Result<(String, PathBuf), FileCacheError> {
        if file.byte_count < 1 || file.byte_count > self.maximum_bytes {
            return Err(FileCacheError::Capacity);
        }
        let key = hex::encode(file.digest);
        let path = self.root.join(format!("{key}.parquet"));
        let mut state = self.state.lock().await;
        if let Some(entry) = state.entries.get_mut(&key) {
            if valid_file(&entry.path, entry.size, &file.digest)? {
                entry.references += 1;
                entry.last_used = Instant::now();
                return Ok((key, entry.path.clone()));
            }
            let size = entry.size;
            state.entries.remove(&key);
            state.total_bytes -= size;
        }
        ensure_capacity(&mut state, self.maximum_bytes, file.byte_count)?;
        // Holding this per-cache async lock deliberately coalesces identical
        // downloads. Query concurrency is separately bounded to one worker.
        let body = self.objects.get(&file.object_key).await?;
        if i64::try_from(body.len()).map_err(|_| FileCacheError::Integrity)? != file.byte_count
            || Sha256::digest(&body).as_slice() != file.digest
        {
            return Err(FileCacheError::Integrity);
        }
        let temporary = self
            .root
            .join(format!(".{key}.{}.tmp", uuid::Uuid::new_v4()));
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        output.write_all(&body)?;
        output.sync_all()?;
        fs::set_permissions(&temporary, private_permissions())?;
        if let Err(error) = fs::rename(&temporary, &path) {
            let _ = fs::remove_file(&temporary);
            return Err(FileCacheError::Filesystem(error));
        }
        state.total_bytes += file.byte_count;
        state.entries.insert(
            key.clone(),
            Entry {
                path: path.clone(),
                size: file.byte_count,
                references: 1,
                last_used: Instant::now(),
            },
        );
        Ok((key, path))
    }

    async fn release_keys(&self, keys: &[String]) {
        let mut state = self.state.lock().await;
        for key in keys {
            if let Some(entry) = state.entries.get_mut(key) {
                entry.references = entry.references.saturating_sub(1);
                entry.last_used = Instant::now();
            }
        }
    }
}

fn ensure_capacity(state: &mut State, maximum: i64, incoming: i64) -> Result<(), FileCacheError> {
    if state.total_bytes + incoming <= maximum {
        return Ok(());
    }
    let mut candidates: Vec<(String, Instant)> = state
        .entries
        .iter()
        .filter(|(_, entry)| entry.references == 0)
        .map(|(key, entry)| (key.clone(), entry.last_used))
        .collect();
    candidates.sort_by_key(|(_, last_used)| *last_used);
    for (key, _) in candidates {
        if let Some(entry) = state.entries.remove(&key) {
            let _ = fs::remove_file(&entry.path);
            state.total_bytes -= entry.size;
        }
        if state.total_bytes + incoming <= maximum {
            return Ok(());
        }
    }
    Err(FileCacheError::Capacity)
}

fn valid_file(path: &Path, size: i64, digest: &[u8; 32]) -> Result<bool, std::io::Error> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file() || i64::try_from(metadata.len()).ok() != Some(size) {
        return Ok(false);
    }
    Ok(Sha256::digest(fs::read(path)?).as_slice() == digest)
}

#[cfg(unix)]
fn private_permissions() -> fs::Permissions {
    use std::os::unix::fs::PermissionsExt as _;
    fs::Permissions::from_mode(0o600)
}

#[cfg(unix)]
fn private_directory_permissions() -> fs::Permissions {
    use std::os::unix::fs::PermissionsExt as _;
    fs::Permissions::from_mode(0o700)
}

#[cfg(not(unix))]
fn private_permissions() -> fs::Permissions {
    fs::Permissions::readonly()
}

#[cfg(not(unix))]
fn private_directory_permissions() -> fs::Permissions {
    fs::Permissions::readonly()
}
