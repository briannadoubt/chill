//! Immutable, digest-checked filesystem and S3 object storage.

use std::{env, fs, path::Path as FilePath, sync::Arc};

use aws_config::{BehaviorVersion, Region};
use aws_sdk_s3::Client as S3Client;
use bytes::Bytes;
use object_store::{
    ObjectStore, ObjectStoreExt as _, PutMode,
    aws::{AmazonS3Builder, AmazonS3ConfigKey, S3ConditionalPut},
    local::LocalFileSystem,
    path::Path,
};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

/// Stable object-boundary failure.
#[derive(Debug, Error)]
pub enum ObjectError {
    /// The object key violates the safe relative-path contract.
    #[error("object key is invalid")]
    InvalidKey,
    /// Caller-provided content does not match its digest.
    #[error("object body digest mismatch")]
    DigestMismatch,
    /// An immutable key already names different content.
    #[error("immutable object already exists with different content")]
    ImmutableConflict,
    /// Environment or backend configuration is invalid.
    #[error("object-store configuration is invalid: {0}")]
    InvalidConfiguration(String),
    /// Filesystem setup failed.
    #[error("configure filesystem object store: {0}")]
    Filesystem(#[from] std::io::Error),
    /// Backend operation failed.
    #[error("object-store operation failed: {0}")]
    Backend(#[from] object_store::Error),
    /// Version-aware S3 erasure failed.
    #[error("S3 version erasure failed: {0}")]
    S3Erasure(String),
}

/// Cloneable immutable object boundary shared by lake and privacy workers.
#[derive(Clone)]
pub struct ImmutableStore {
    inner: Arc<dyn ObjectStore>,
    s3_eraser: Option<Arc<S3VersionEraser>>,
}

struct S3VersionEraser {
    bucket: String,
    client: S3Client,
}

impl ImmutableStore {
    /// Creates a digest-checked wrapper over an object-store implementation.
    #[must_use]
    pub fn new(inner: Arc<dyn ObjectStore>) -> Self {
        Self {
            inner,
            s3_eraser: None,
        }
    }

    /// Creates a local store rooted below a canonicalized directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the root is empty, cannot be secured/created, or is invalid.
    pub fn filesystem(root: impl AsRef<FilePath>) -> Result<Self, ObjectError> {
        if root.as_ref().as_os_str().is_empty() {
            return Err(ObjectError::InvalidConfiguration(
                "filesystem root is required".to_owned(),
            ));
        }
        fs::create_dir_all(root.as_ref())?;
        let canonical = fs::canonicalize(root.as_ref())?;
        let store = LocalFileSystem::new_with_prefix(canonical)?;
        Ok(Self::new(Arc::new(store)))
    }

    /// Builds the configured filesystem or S3 store from Chill environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported backends, invalid booleans, missing S3 bucket,
    /// unsafe HTTP configuration, or backend construction failures.
    pub async fn from_environment() -> Result<Self, ObjectError> {
        match environment("CHILL_OBJECT_STORE_BACKEND")
            .unwrap_or_else(|| "filesystem".to_owned())
            .to_ascii_lowercase()
            .as_str()
        {
            "filesystem" => Self::filesystem(
                environment("CHILL_OBJECT_STORE_PATH")
                    .unwrap_or_else(|| ".chill-data/objects".to_owned()),
            ),
            "s3" => Self::s3_from_environment().await,
            _ => Err(ObjectError::InvalidConfiguration(
                "CHILL_OBJECT_STORE_BACKEND must be filesystem or s3".to_owned(),
            )),
        }
    }

    async fn s3_from_environment() -> Result<Self, ObjectError> {
        let bucket = environment("CHILL_S3_BUCKET").ok_or_else(|| {
            ObjectError::InvalidConfiguration("CHILL_S3_BUCKET is required".to_owned())
        })?;
        let region = environment("CHILL_S3_REGION").unwrap_or_else(|| "us-east-1".to_owned());
        let path_style = environment("CHILL_S3_PATH_STYLE")
            .map(|value| {
                value.parse::<bool>().map_err(|_| {
                    ObjectError::InvalidConfiguration(
                        "CHILL_S3_PATH_STYLE must be a boolean".to_owned(),
                    )
                })
            })
            .transpose()?
            .unwrap_or(true);
        let endpoint = environment("CHILL_S3_ENDPOINT");
        let mut builder = AmazonS3Builder::from_env()
            .with_bucket_name(&bucket)
            .with_region(&region)
            .with_virtual_hosted_style_request(!path_style)
            .with_conditional_put(S3ConditionalPut::ETagMatch);
        if let Some(endpoint) = endpoint.as_ref() {
            let allow_http = endpoint.starts_with("http://");
            let endpoint = object_store_endpoint(endpoint, &bucket, path_style)?;
            builder = builder
                .with_config(AmazonS3ConfigKey::S3Endpoint, endpoint)
                .with_allow_http(allow_http);
        }
        let store = builder
            .build()
            .map_err(|error| ObjectError::InvalidConfiguration(error.to_string()))?;
        let shared = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(region))
            .load()
            .await;
        let mut s3_configuration =
            aws_sdk_s3::config::Builder::from(&shared).force_path_style(path_style);
        if let Some(endpoint) = endpoint {
            s3_configuration = s3_configuration.endpoint_url(endpoint);
        }
        Ok(Self {
            inner: Arc::new(store),
            s3_eraser: Some(Arc::new(S3VersionEraser {
                bucket,
                client: S3Client::from_conf(s3_configuration.build()),
            })),
        })
    }

    /// Atomically creates an immutable object, accepting identical retries.
    ///
    /// # Errors
    ///
    /// Returns a validation, conflict, or backend error.
    pub async fn put_if_absent(
        &self,
        key: &str,
        body: Bytes,
        digest: [u8; 32],
    ) -> Result<(), ObjectError> {
        verify_digest(&body, digest)?;
        let path = object_path(key)?;
        match self
            .inner
            .put_opts(&path, body.clone().into(), PutMode::Create.into())
            .await
        {
            Ok(_) => Ok(()),
            Err(
                object_store::Error::AlreadyExists { .. }
                | object_store::Error::Precondition { .. },
            ) => {
                let existing = self.get(key).await?;
                if Sha256::digest(&existing).as_slice() == digest {
                    Ok(())
                } else {
                    Err(ObjectError::ImmutableConflict)
                }
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Reads one complete immutable object.
    ///
    /// # Errors
    ///
    /// Returns a validation or backend error.
    pub async fn get(&self, key: &str) -> Result<Bytes, ObjectError> {
        Ok(self.inner.get(&object_path(key)?).await?.bytes().await?)
    }

    /// Deletes the current object. Version eradication is performed by the
    /// audited lifecycle backend before this boundary reports completion.
    ///
    /// # Errors
    ///
    /// Returns a validation or backend error. A missing object is idempotent.
    pub async fn delete(&self, key: &str) -> Result<(), ObjectError> {
        validate_key(key)?;
        if let Some(eraser) = &self.s3_eraser {
            return eraser.delete_all_versions(key).await;
        }
        let path = object_path(key)?;
        match self.inner.delete(&path).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

fn object_store_endpoint(
    endpoint: &str,
    bucket: &str,
    path_style: bool,
) -> Result<String, ObjectError> {
    let endpoint = endpoint.trim_end_matches('/');
    if path_style {
        return Ok(endpoint.to_owned());
    }
    let (scheme, authority) = endpoint.split_once("://").ok_or_else(|| {
        ObjectError::InvalidConfiguration("CHILL_S3_ENDPOINT must be an HTTP(S) URL".to_owned())
    })?;
    if !matches!(scheme, "http" | "https")
        || authority.is_empty()
        || authority.contains(['/', '?', '#'])
    {
        return Err(ObjectError::InvalidConfiguration(
            "CHILL_S3_ENDPOINT must contain only an HTTP(S) origin".to_owned(),
        ));
    }
    if authority == bucket || authority.starts_with(&format!("{bucket}.")) {
        return Ok(endpoint.to_owned());
    }
    Ok(format!("{scheme}://{bucket}.{authority}"))
}

impl S3VersionEraser {
    async fn delete_all_versions(&self, key: &str) -> Result<(), ObjectError> {
        let versions = self.object_versions(key).await?;
        if versions.is_empty() {
            self.client
                .delete_object()
                .bucket(&self.bucket)
                .key(key)
                .send()
                .await
                .map_err(s3_error)?;
        } else {
            for version in versions {
                self.client
                    .delete_object()
                    .bucket(&self.bucket)
                    .key(key)
                    .version_id(version)
                    .send()
                    .await
                    .map_err(s3_error)?;
            }
        }
        if self.object_versions(key).await?.is_empty() {
            Ok(())
        } else {
            Err(ObjectError::S3Erasure(
                "object versions remain after deletion".to_owned(),
            ))
        }
    }

    async fn object_versions(&self, key: &str) -> Result<Vec<String>, ObjectError> {
        let mut versions = Vec::new();
        let mut key_marker = None;
        let mut version_marker = None;
        loop {
            let mut request = self
                .client
                .list_object_versions()
                .bucket(&self.bucket)
                .prefix(key);
            if let Some(marker) = key_marker.as_deref() {
                request = request.key_marker(marker);
            }
            if let Some(marker) = version_marker.as_deref() {
                request = request.version_id_marker(marker);
            }
            let result = request.send().await.map_err(s3_error)?;
            for version in result.versions() {
                if version.key() == Some(key)
                    && let Some(version_id) = version.version_id()
                {
                    versions.push(version_id.to_owned());
                }
            }
            for marker in result.delete_markers() {
                if marker.key() == Some(key)
                    && let Some(version_id) = marker.version_id()
                {
                    versions.push(version_id.to_owned());
                }
            }
            if !result.is_truncated().unwrap_or(false) {
                return Ok(versions);
            }
            key_marker = result.next_key_marker().map(str::to_owned);
            version_marker = result.next_version_id_marker().map(str::to_owned);
            if key_marker.is_none() || version_marker.is_none() {
                return Err(ObjectError::S3Erasure(
                    "version listing was truncated without continuation markers".to_owned(),
                ));
            }
        }
    }
}

fn s3_error(error: impl std::fmt::Display) -> ObjectError {
    ObjectError::S3Erasure(error.to_string())
}

/// Validates one safe relative object key.
///
/// # Errors
///
/// Rejects empty, absolute, oversized, backslash/NUL, dot, and empty segments.
pub fn validate_key(key: &str) -> Result<(), ObjectError> {
    if key.is_empty()
        || key.len() > 1024
        || key.starts_with('/')
        || key.contains(['\\', '\0'])
        || key
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(ObjectError::InvalidKey);
    }
    Ok(())
}

fn object_path(key: &str) -> Result<Path, ObjectError> {
    validate_key(key)?;
    Path::parse(key).map_err(|_| ObjectError::InvalidKey)
}

fn verify_digest(body: &[u8], expected: [u8; 32]) -> Result<(), ObjectError> {
    if Sha256::digest(body).as_slice() == expected {
        Ok(())
    } else {
        Err(ObjectError::DigestMismatch)
    }
}

fn environment(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use sha2::{Digest as _, Sha256};

    use super::{ImmutableStore, ObjectError, object_store_endpoint, validate_key};

    #[test]
    fn custom_s3_endpoint_matches_request_addressing_style() {
        assert_eq!(
            object_store_endpoint("https://t3.storage.dev", "chill-production", false)
                .unwrap_or_default(),
            "https://chill-production.t3.storage.dev"
        );
        assert_eq!(
            object_store_endpoint(
                "https://chill-production.t3.storage.dev/",
                "chill-production",
                false,
            )
            .unwrap_or_default(),
            "https://chill-production.t3.storage.dev"
        );
        assert_eq!(
            object_store_endpoint("http://localhost:9000/", "chill-production", true)
                .unwrap_or_default(),
            "http://localhost:9000"
        );
        assert!(
            object_store_endpoint("https://t3.storage.dev/path", "chill-production", false,)
                .is_err()
        );
    }

    #[tokio::test]
    async fn filesystem_is_immutable_idempotent_and_path_safe() {
        let root = std::env::temp_dir().join(format!("chill-objects-{}", uuid::Uuid::new_v4()));
        let store = ImmutableStore::filesystem(&root).unwrap_or_else(|error| {
            unreachable!("temporary filesystem object store should initialize: {error}")
        });
        let body = Bytes::from_static(b"immutable");
        let digest = Sha256::digest(&body).into();
        assert!(
            store
                .put_if_absent("safe/object", body.clone(), digest)
                .await
                .is_ok()
        );
        assert!(
            store
                .put_if_absent("safe/object", body, digest)
                .await
                .is_ok()
        );
        let conflict = Bytes::from_static(b"different");
        let conflict_digest = Sha256::digest(&conflict).into();
        assert!(matches!(
            store
                .put_if_absent("safe/object", conflict, conflict_digest)
                .await,
            Err(ObjectError::ImmutableConflict)
        ));
        assert!(validate_key("../escape").is_err());
        assert!(store.delete("safe/object").await.is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    #[ignore = "requires CHILL_TEST_S3_ENDPOINT and configured MinIO/S3 credentials"]
    async fn s3_is_immutable_idempotent_and_deletable() {
        assert!(
            std::env::var("CHILL_TEST_S3_ENDPOINT").is_ok(),
            "CHILL_TEST_S3_ENDPOINT is required"
        );
        let store = ImmutableStore::from_environment()
            .await
            .unwrap_or_else(|error| {
                unreachable!("configured S3 object store should initialize: {error}")
            });
        let key = format!("rust-object-tests/{}/immutable", uuid::Uuid::new_v4());
        let body = Bytes::from_static(b"immutable-s3");
        let digest = Sha256::digest(&body).into();
        assert!(
            store
                .put_if_absent(&key, body.clone(), digest)
                .await
                .is_ok()
        );
        assert!(store.put_if_absent(&key, body, digest).await.is_ok());
        assert_eq!(
            store.get(&key).await.unwrap_or_default(),
            Bytes::from_static(b"immutable-s3")
        );
        let conflict = Bytes::from_static(b"different-s3");
        let conflict_digest = Sha256::digest(&conflict).into();
        assert!(matches!(
            store.put_if_absent(&key, conflict, conflict_digest).await,
            Err(ObjectError::ImmutableConflict)
        ));
        assert!(store.delete(&key).await.is_ok());
    }
}
