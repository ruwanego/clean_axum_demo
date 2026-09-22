//! File storage abstraction (12-factor: backing services as attached resources).
//!
//! Domain code depends only on the `FileStorage` trait. The concrete backend is
//! chosen by configuration: `STORAGE_BACKEND=local` stores files under
//! `ASSETS_PRIVATE_PATH`, `STORAGE_BACKEND=s3` stores them in `S3_BUCKET` using the
//! standard `AWS_*` environment variables (works with S3, MinIO, R2, ...).

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt, stream::BoxStream};
use object_store::{
    ObjectStore, ObjectStoreExt, PutPayload, aws::AmazonS3Builder, local::LocalFileSystem,
    path::Path,
};

use super::{
    config::{Config, StorageBackend},
    error::AppError,
};

/// A stream of file bytes, suitable for `axum::body::Body::from_stream`.
pub type ByteStream = BoxStream<'static, Result<Bytes, StorageError>>;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("object not found: {0}")]
    NotFound(String),
    #[error("invalid object key {0:?}")]
    InvalidKey(String),
    #[error("storage backend error: {0}")]
    Backend(#[from] object_store::Error),
}

impl From<StorageError> for AppError {
    fn from(err: StorageError) -> Self {
        match err {
            StorageError::NotFound(_) | StorageError::InvalidKey(_) => {
                AppError::NotFound("File not found".into())
            }
            StorageError::Backend(err) => {
                tracing::error!("Storage error: {err}");
                AppError::InternalError
            }
        }
    }
}

/// Storage for uploaded files, addressed by a relative key such as
/// `profile_picture/5f0c....png`.
#[async_trait]
pub trait FileStorage: Send + Sync {
    /// Stores `data` under `key`, replacing any existing object.
    async fn put(&self, key: &str, data: Bytes) -> Result<(), StorageError>;

    /// Streams the object stored under `key`.
    async fn get(&self, key: &str) -> Result<ByteStream, StorageError>;

    /// Deletes the object stored under `key`.
    async fn delete(&self, key: &str) -> Result<(), StorageError>;
}

/// `FileStorage` backed by any `object_store` implementation (local disk or S3).
pub struct ObjectFileStorage {
    store: Arc<dyn ObjectStore>,
}

impl ObjectFileStorage {
    pub fn new(store: Arc<dyn ObjectStore>) -> Self {
        Self { store }
    }

    /// Parses a key, rejecting empty keys and `.`/`..` segments (no path traversal).
    fn path(key: &str) -> Result<Path, StorageError> {
        let path = Path::parse(key).map_err(|_| StorageError::InvalidKey(key.to_string()))?;
        if path.as_ref().is_empty() {
            return Err(StorageError::InvalidKey(key.to_string()));
        }
        Ok(path)
    }
}

fn map_err(key: &str, err: object_store::Error) -> StorageError {
    match err {
        object_store::Error::NotFound { .. } => StorageError::NotFound(key.to_string()),
        other => StorageError::Backend(other),
    }
}

#[async_trait]
impl FileStorage for ObjectFileStorage {
    async fn put(&self, key: &str, data: Bytes) -> Result<(), StorageError> {
        let path = Self::path(key)?;
        self.store
            .put(&path, PutPayload::from_bytes(data))
            .await
            .map_err(|e| map_err(key, e))?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<ByteStream, StorageError> {
        let path = Self::path(key)?;
        let result = self.store.get(&path).await.map_err(|e| map_err(key, e))?;
        Ok(result.into_stream().map_err(StorageError::from).boxed())
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let path = Self::path(key)?;
        self.store.delete(&path).await.map_err(|e| map_err(key, e))
    }
}

/// Builds the storage backend selected by `STORAGE_BACKEND`.
pub fn build_storage(config: &Config) -> Result<Arc<dyn FileStorage>, StorageError> {
    let store: Arc<dyn ObjectStore> = match &config.storage_backend {
        StorageBackend::Local => {
            // LocalFileSystem requires the root directory to exist.
            std::fs::create_dir_all(&config.assets_private_path).map_err(|e| {
                StorageError::Backend(object_store::Error::Generic {
                    store: "LocalFileSystem",
                    source: Box::new(e),
                })
            })?;
            Arc::new(LocalFileSystem::new_with_prefix(
                &config.assets_private_path,
            )?)
        }
        StorageBackend::S3 { bucket } => Arc::new(
            AmazonS3Builder::from_env()
                .with_bucket_name(bucket)
                .build()?,
        ),
    };
    Ok(Arc::new(ObjectFileStorage::new(store)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::memory::InMemory;

    fn storage() -> ObjectFileStorage {
        ObjectFileStorage::new(Arc::new(InMemory::new()))
    }

    async fn read_all(storage: &ObjectFileStorage, key: &str) -> Vec<u8> {
        let chunks: Vec<Bytes> = storage.get(key).await.unwrap().try_collect().await.unwrap();
        chunks.concat()
    }

    #[tokio::test]
    async fn put_get_delete_roundtrip() {
        let storage = storage();
        storage
            .put("profile_picture/a.png", Bytes::from_static(b"png"))
            .await
            .unwrap();

        assert_eq!(read_all(&storage, "profile_picture/a.png").await, b"png");

        storage.delete("profile_picture/a.png").await.unwrap();
        assert!(matches!(
            storage.get("profile_picture/a.png").await,
            Err(StorageError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn rejects_path_traversal_and_empty_keys() {
        let storage = storage();
        for key in ["../etc/passwd", "a/../../b", ""] {
            assert!(
                matches!(
                    storage.put(key, Bytes::new()).await,
                    Err(StorageError::InvalidKey(_))
                ),
                "{key:?} should be rejected"
            );
        }
    }
}
