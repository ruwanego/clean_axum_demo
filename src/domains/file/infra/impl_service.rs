use crate::common::{
    config::Config,
    error::AppError,
    storage::{ByteStream, FileStorage},
};
use crate::domains::file::domain::model::FileType;
use crate::domains::file::domain::repository::FileRepository;
use crate::domains::file::domain::service::FileServiceTrait;
use crate::domains::file::dto::file_dto::{CreateFileDto, UploadFileDto, UploadedFileDto};
use crate::domains::file::infra::impl_repository::FileRepo;

use bytes::Bytes;
use sqlx::{PgPool, Postgres, Transaction};
use std::path::Path as FilePath;
use std::sync::Arc;
use uuid::Uuid;

use async_trait::async_trait;

/// Service struct for handling file-related operations
/// such as uploading, deleting, and fetching files.
/// It uses a repository pattern to abstract the data access layer.
#[derive(Clone)]
pub struct FileService {
    config: Config,
    pool: PgPool,
    repo: Arc<dyn FileRepository + Send + Sync>,
    storage: Arc<dyn FileStorage>,
}

/// Implementation of the FileService struct
#[async_trait]
impl FileServiceTrait for FileService {
    /// constructor for the service.
    fn create_service(
        config: Config,
        pool: PgPool,
        storage: Arc<dyn FileStorage>,
    ) -> Arc<dyn FileServiceTrait> {
        Arc::new(Self {
            config,
            pool,
            repo: Arc::new(FileRepo {}),
            storage,
        })
    }

    /// Uploads a profile picture for a user.
    /// Validates the file, writes it to storage, and stores its metadata in the database.
    /// Returns the uploaded file's metadata.
    async fn process_profile_picture_upload(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        upload_file_dto: &UploadFileDto,
    ) -> Result<Option<UploadedFileDto>, AppError> {
        let file_dto = &upload_file_dto.file;

        if file_dto.data.is_empty() {
            tracing::error!("File data is empty.");
            return Err(AppError::InvalidFileData);
        }

        let (unique_filename, file_relative_path) =
            Self::build_storage_key(&file_dto.original_filename);

        self.storage
            .put(&file_relative_path, Bytes::copy_from_slice(&file_dto.data))
            .await?;

        let file_url = format!("{}/{}", self.config.assets_private_url, file_relative_path);

        let create_file_dto = CreateFileDto {
            user_id: upload_file_dto.user_id.clone(),
            file_name: unique_filename,
            origin_file_name: file_dto.original_filename.clone(),
            file_relative_path,
            file_url,
            content_type: file_dto.content_type.clone(),
            file_size: file_dto.data.len() as u32,
            file_type: FileType::ProfilePicture,
            modified_by: upload_file_dto.modified_by.clone(),
        };

        self.repo
            .create_file(tx, create_file_dto)
            .await
            .map_err(|err| {
                tracing::error!("Error uploading file: {}", err);
                AppError::DatabaseError(err)
            })?;

        if let Some(user_id) = &upload_file_dto.user_id {
            self.get_file_by_user(user_id.clone()).await
        } else {
            Err(AppError::ValidationError("User ID is missing".into()))
        }
    }

    /// Retrieves the metadata of a file by its id.
    async fn get_file_metadata(
        &self,
        file_id: String,
    ) -> Result<Option<UploadedFileDto>, AppError> {
        let uploaded_file = self
            .repo
            .find_by_id(self.pool.clone(), file_id.clone())
            .await
            .map_err(|err| {
                tracing::error!("Error retrieving file: {}", err);
                AppError::DatabaseError(err)
            });

        match uploaded_file {
            Ok(Some(file)) => Ok(Some(UploadedFileDto::from(file))),
            Ok(None) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Deletes a file by its id.
    /// Removes the file from storage and deletes its metadata from the database.
    /// Returns a success message if the deletion was successful.
    async fn delete_file(&self, file_id: String) -> Result<String, AppError> {
        let mut tx = self.pool.begin().await?;

        let to_delete_file = self
            .repo
            .find_by_id(self.pool.clone(), file_id.clone())
            .await
            .map_err(|err| {
                tracing::error!("Error retrieving file: {}", err);
                AppError::DatabaseError(err)
            })?;

        if to_delete_file.is_none() {
            return Err(AppError::NotFound("File not found".into()));
        }

        let deletion_result = self.repo.delete(&mut tx, file_id).await.map_err(|err| {
            tracing::error!("Error deleting file: {}", err);
            AppError::DatabaseError(err)
        })?;

        if !deletion_result {
            return Err(AppError::NotFound("File not found".into()));
        }

        if let Some(file) = to_delete_file {
            self.storage.delete(&file.file_relative_path).await?;
        }

        tx.commit().await?;

        Ok("File deleted successfully".into())
    }

    async fn read_file(&self, relative_path: &str) -> Result<ByteStream, AppError> {
        Ok(self.storage.get(relative_path).await?)
    }
}

/// Internal helper methods defined on `FileService`.
impl FileService {
    /// Retrieves file metadata associated with a given user ID from the repository.
    async fn get_file_by_user(&self, user_id: String) -> Result<Option<UploadedFileDto>, AppError> {
        let uploaded_file = self
            .repo
            .find_by_user_id(self.pool.clone(), user_id)
            .await
            .map_err(|err| {
                tracing::error!("Error retrieving file: {}", err);
                AppError::DatabaseError(err)
            });

        match uploaded_file {
            Ok(Some(file)) => Ok(Some(UploadedFileDto::from(file))),
            Ok(None) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Builds a collision-free storage key for an upload: `profile_picture/<uuid>.<ext>`.
    /// Returns `(file_name, relative_key)`. The original name is kept only as metadata,
    /// so it never influences the storage path.
    fn build_storage_key(original_filename: &str) -> (String, String) {
        let ext = FilePath::new(original_filename)
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric()))
            .map(str::to_ascii_lowercase);

        let file_name = match ext {
            Some(ext) => format!("{}.{ext}", Uuid::new_v4()),
            None => Uuid::new_v4().to_string(),
        };
        let relative_key = format!("{}/{}", FileType::ProfilePicture, file_name);
        (file_name, relative_key)
    }
}

#[cfg(test)]
mod tests {
    use super::FileService;

    #[test]
    fn storage_key_is_unique_and_ignores_directories() {
        let (name_a, key_a) = FileService::build_storage_key("../../etc/Cat.PNG");
        let (name_b, _) = FileService::build_storage_key("../../etc/Cat.PNG");

        assert_ne!(name_a, name_b);
        assert!(name_a.ends_with(".png"));
        assert_eq!(key_a, format!("profile_picture/{name_a}"));
    }

    #[test]
    fn storage_key_drops_suspicious_extensions() {
        let (name, _) = FileService::build_storage_key("evil.p/hp");
        assert!(!name.contains('/'));
        let (name, _) = FileService::build_storage_key("noext");
        assert!(!name.contains('.'));
    }
}
