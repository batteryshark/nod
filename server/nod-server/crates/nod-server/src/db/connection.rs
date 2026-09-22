use std::{path::Path, str::FromStr};

use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    Row, SqlitePool,
};
use url::Url;

use crate::config::Config;

pub async fn connect(config: &Config) -> anyhow::Result<SqlitePool> {
    tokio::fs::create_dir_all(&config.data_dir).await?;
    ensure_sqlite_parent(&config.database_url)?;

    let options = SqliteConnectOptions::from_str(&config.database_url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(10)
        .connect_with(options)
        .await?;
    create_greenfield_schema(&pool).await?;
    Ok(pool)
}

async fn create_greenfield_schema(pool: &SqlitePool) -> anyhow::Result<()> {
    // Greenfield installs apply one idempotent schema instead of ordering historical migrations.
    sqlx::raw_sql(include_str!("schema.sql"))
        .execute(pool)
        .await?;
    migrate_existing_databases(pool).await?;
    seed_defaults(pool).await?;
    Ok(())
}

async fn migrate_existing_databases(pool: &SqlitePool) -> anyhow::Result<()> {
    // Additive migrations preserve installed enrollments and the original v1
    // request snapshots. The random salt is never included in device views.
    for (table, column, definition) in [
        ("users", "deleted_at", "TEXT"),
        ("devices", "revoked_at", "TEXT"),
        (
            "devices",
            "notification_preferences_json",
            "TEXT NOT NULL DEFAULT '{}'",
        ),
        ("requests", "recipient_salt", "TEXT NOT NULL DEFAULT ''"),
        (
            "requests",
            "explicit_recipients",
            "INTEGER NOT NULL DEFAULT 0",
        ),
    ] {
        let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(pool)
            .await?;
        if !rows
            .iter()
            .any(|row| row.get::<String, _>("name") == column)
        {
            sqlx::query(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition}"
            ))
            .execute(pool)
            .await?;
        }
    }
    sqlx::query(
        "UPDATE requests SET recipient_salt = lower(hex(randomblob(32))) WHERE recipient_salt = ''",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_defaults(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO channels (id, name, emoji, created_at) VALUES ('default', 'Default', '🔔', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO users (id, name, created_at, updated_at)
        VALUES (
            'owner',
            'Owner',
            strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        )
        "#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO user_channel_subscriptions (user_id, channel_id, subscribed, updated_at)
        VALUES ('owner', 'default', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        "#,
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn ensure_sqlite_parent(database_url: &str) -> anyhow::Result<()> {
    if database_url == "sqlite::memory:" || database_url.contains("mode=memory") {
        return Ok(());
    }
    if let Some(path) = database_url.strip_prefix("sqlite://") {
        let path = path.split('?').next().unwrap_or(path);
        if let Some(parent) = Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
    } else if let Ok(url) = Url::parse(database_url) {
        if url.scheme() == "sqlite" {
            if let Some(path) = url.path().strip_prefix('/') {
                if let Some(parent) = Path::new(path).parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn additive_upgrade_preserves_installed_enrollment_and_stable_commitment_salt() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = Config::with_admin_token("test-admin");
        config.data_dir = directory.path().join("data");
        config.database_url = format!("sqlite://{}", directory.path().join("old.sqlite").display());
        let options = SqliteConnectOptions::from_str(&config.database_url)
            .unwrap()
            .create_if_missing(true);
        let old = SqlitePool::connect_with(options).await.unwrap();
        let previous_schema = include_str!("schema.sql")
            .replace(
                "    updated_at TEXT NOT NULL,\n    deleted_at TEXT",
                "    updated_at TEXT NOT NULL",
            )
            .replace("    revoked_at TEXT,\n", "")
            .replace(
                "    notification_preferences_json TEXT NOT NULL DEFAULT '{}',\n",
                "",
            )
            .replace("    recipient_salt TEXT NOT NULL DEFAULT '',\n", "")
            .replace("    explicit_recipients INTEGER NOT NULL DEFAULT 0,\n", "");
        sqlx::raw_sql(&previous_schema).execute(&old).await.unwrap();
        seed_defaults(&old).await.unwrap();
        sqlx::query("INSERT INTO devices(id,user_id,name,platform,token_hash,signing_public_key,last_seen_at,created_at) VALUES('installed','owner','Phone','ios','preserved-token-hash','preserved-public-key','2026-09-01T00:00:00.000Z','2026-09-01T00:00:00.000Z')")
            .execute(&old).await.unwrap();
        sqlx::query("INSERT INTO requests(id,channel_id,title,summary,body_markdown,fields_json,links_json,status,created_at,updated_at) VALUES('old-request','default','Old','','','[]','[]','pending','2026-09-01T00:00:00.000Z','2026-09-01T00:00:00.000Z')")
            .execute(&old).await.unwrap();
        old.close().await;
        let upgraded = connect(&config).await.unwrap();
        let device: (String, String, Option<String>) = sqlx::query_as(
            "SELECT token_hash,signing_public_key,revoked_at FROM devices WHERE id='installed'",
        )
        .fetch_one(&upgraded)
        .await
        .unwrap();
        assert_eq!(
            device,
            (
                "preserved-token-hash".to_string(),
                "preserved-public-key".to_string(),
                None
            )
        );
        let salt: String =
            sqlx::query_scalar("SELECT recipient_salt FROM requests WHERE id='old-request'")
                .fetch_one(&upgraded)
                .await
                .unwrap();
        assert_eq!(salt.len(), 64);
        migrate_existing_databases(&upgraded).await.unwrap();
        let retained: String =
            sqlx::query_scalar("SELECT recipient_salt FROM requests WHERE id='old-request'")
                .fetch_one(&upgraded)
                .await
                .unwrap();
        assert_eq!(retained, salt);
    }
}
