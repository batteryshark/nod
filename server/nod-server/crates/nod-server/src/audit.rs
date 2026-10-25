use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::Utc;
use serde::Serialize;
#[cfg(test)]
use serde_json::json;
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::Mutex};

const ROTATE_AFTER_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct AuditLogger {
    inner: Arc<Mutex<AuditFile>>,
}

struct AuditFile {
    file: tokio::fs::File,
    path: PathBuf,
    bytes: u64,
    rotate_after: u64,
    last_error: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct AuditHealth {
    healthy: bool,
    last_error: Option<String>,
}

impl AuditLogger {
    pub async fn new(data_dir: PathBuf) -> anyhow::Result<Self> {
        Self::with_rotation_limit(data_dir, ROTATE_AFTER_BYTES).await
    }

    async fn with_rotation_limit(data_dir: PathBuf, rotate_after: u64) -> anyhow::Result<Self> {
        let audit_dir = data_dir.join("audit");
        crate::db::create_private_dir_all(&audit_dir)?;
        let path = audit_dir.join("nod.audit.jsonl");
        let file = open_private_append(&path).await?;
        let bytes = file.metadata().await?.len();
        Ok(Self {
            inner: Arc::new(Mutex::new(AuditFile {
                file,
                path,
                bytes,
                rotate_after,
                last_error: None,
            })),
        })
    }

    pub(crate) async fn health(&self) -> AuditHealth {
        let file = self.inner.lock().await;
        AuditHealth {
            healthy: file.last_error.is_none(),
            last_error: file.last_error.clone(),
        }
    }

    pub async fn record<T: Serialize + ?Sized>(&self, kind: &str, payload: &T) {
        #[derive(Serialize)]
        struct Entry<'a, T: Serialize + ?Sized> {
            at: chrono::DateTime<Utc>,
            kind: &'a str,
            payload: &'a T,
        }
        let encoded = serde_json::to_vec(&Entry {
            at: Utc::now(),
            kind,
            payload,
        });
        let mut file = self.inner.lock().await;
        let result = match encoded {
            Ok(mut raw) => {
                raw.push(b'\n');
                file.append(&raw).await
            }
            Err(error) => Err(error.into()),
        };
        if let Err(error) = result {
            tracing::error!(kind, %error, "audit entry was not persisted");
            // Sticky until restart: a later successful write cannot repair
            // evidence that was lost while the disk was unavailable.
            file.last_error = Some(format!("{}: {error}", Utc::now().to_rfc3339()));
        }
    }
}

/// Open the audit log for append, creating it owner-only (0600) on Unix. The
/// mode only applies at creation; an existing file keeps its permissions.
async fn open_private_append(path: &Path) -> std::io::Result<tokio::fs::File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path).await
}

impl AuditFile {
    async fn append(&mut self, raw: &[u8]) -> anyhow::Result<()> {
        if self.bytes > 0 && self.bytes.saturating_add(raw.len() as u64) > self.rotate_after {
            self.file.flush().await?;
            let archive = self.path.with_file_name(format!(
                "nod.audit.{}.{}.jsonl",
                Utc::now().format("%Y%m%dT%H%M%S"),
                uuid::Uuid::new_v4()
            ));
            tokio::fs::rename(&self.path, archive).await?;
            self.file = open_private_append(&self.path).await?;
            self.bytes = 0;
        }
        self.file.write_all(raw).await?;
        self.file.flush().await?;
        self.bytes = self.bytes.saturating_add(raw.len() as u64);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rotation_preserves_every_entry_in_parseable_archives() {
        let directory = tempfile::tempdir().unwrap();
        let logger = AuditLogger::with_rotation_limit(directory.path().to_path_buf(), 160)
            .await
            .unwrap();
        for index in 0..8 {
            logger
                .record("decision.recorded", &json!({"index":index}))
                .await;
        }
        let mut files = tokio::fs::read_dir(directory.path().join("audit"))
            .await
            .unwrap();
        let mut indices = Vec::new();
        let mut count = 0;
        while let Some(file) = files.next_entry().await.unwrap() {
            count += 1;
            for line in tokio::fs::read_to_string(file.path())
                .await
                .unwrap()
                .lines()
            {
                let record: serde_json::Value = serde_json::from_str(line).unwrap();
                indices.push(record["payload"]["index"].as_u64().unwrap());
            }
        }
        indices.sort_unstable();
        assert_eq!(indices, (0..8).collect::<Vec<_>>());
        assert!(count > 1);
        assert!(logger.health().await.healthy);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn audit_directory_and_file_are_created_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let data_dir = directory.path().join("nested").join("data");
        let logger = AuditLogger::with_rotation_limit(data_dir.clone(), 64)
            .await
            .unwrap();
        for index in 0..4 {
            logger.record("entry", &json!({ "index": index })).await;
        }
        let dir_mode = std::fs::metadata(data_dir.join("audit"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
        let mut files = tokio::fs::read_dir(data_dir.join("audit")).await.unwrap();
        let mut seen = 0;
        while let Some(file) = files.next_entry().await.unwrap() {
            seen += 1;
            let mode = file.metadata().await.unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{}", file.path().display());
        }
        assert!(seen > 1, "rotation should have produced an archive");
    }

    #[tokio::test]
    async fn rotation_failure_is_visible_in_sticky_health() {
        let directory = tempfile::tempdir().unwrap();
        let logger = AuditLogger::with_rotation_limit(directory.path().to_path_buf(), 1)
            .await
            .unwrap();
        logger.record("first", &()).await;
        tokio::fs::remove_file(directory.path().join("audit/nod.audit.jsonl"))
            .await
            .unwrap();
        logger.record("lost", &()).await;
        assert!(!logger.health().await.healthy);
        assert!(logger.health().await.last_error.is_some());
    }
}
