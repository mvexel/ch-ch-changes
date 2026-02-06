use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use std::path::Path;
use std::sync::Arc;
use tokio_rusqlite::Connection;
use tokio_rusqlite::{params, rusqlite, OptionalExtension};

#[derive(Clone)]
pub struct AuthDb {
    conn: Arc<Connection>,
}

#[derive(Clone, Debug)]
pub struct AuthenticatedKey {
    pub api_key_id: i64,
    pub user_id: i64,
    pub osm_uid: i64,
    pub display_name: String,
    pub key_prefix: String,
    pub created_at: String,
    pub daily_limit: Option<i64>,
    pub monthly_limit: Option<i64>,
}

#[derive(Clone, Copy, Debug)]
pub struct UserDefaults {
    pub daily_limit: Option<i64>,
    pub monthly_limit: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct UserRecord {
    pub id: i64,
    pub osm_uid: i64,
    pub display_name: String,
    pub daily_limit: Option<i64>,
    pub monthly_limit: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct UsageSnapshot {
    pub daily_count: i64,
    pub monthly_count: i64,
}

impl AuthDb {
    pub async fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if parent.as_os_str().is_empty() {
                return Self::open(path).await;
            }
            std::fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create auth db directory {}", parent.display())
            })?;
        }
        Self::open(path).await
    }

    async fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .await
            .with_context(|| format!("Failed to open auth db at {}", path.display()))?;
        let db = Self {
            conn: Arc::new(conn),
        };
        db.init().await?;
        Ok(db)
    }

    async fn init(&self) -> Result<()> {
        let conn = self.conn.clone();
        conn.call(|conn| {
            conn.execute_batch(
                "
                PRAGMA journal_mode = WAL;
                PRAGMA foreign_keys = ON;
                CREATE TABLE IF NOT EXISTS users (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    osm_uid INTEGER NOT NULL UNIQUE,
                    display_name TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    daily_limit INTEGER,
                    monthly_limit INTEGER
                );
                CREATE TABLE IF NOT EXISTS api_keys (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    user_id INTEGER NOT NULL,
                    key_prefix TEXT NOT NULL,
                    key_hash TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    revoked_at TEXT,
                    last_used_at TEXT,
                    FOREIGN KEY(user_id) REFERENCES users(id)
                );
                CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON api_keys(key_hash);
                CREATE INDEX IF NOT EXISTS idx_api_keys_user ON api_keys(user_id);
                CREATE TABLE IF NOT EXISTS usage_daily (
                    api_key_id INTEGER NOT NULL,
                    day TEXT NOT NULL,
                    count INTEGER NOT NULL,
                    PRIMARY KEY(api_key_id, day),
                    FOREIGN KEY(api_key_id) REFERENCES api_keys(id)
                );
                CREATE TABLE IF NOT EXISTS usage_monthly (
                    api_key_id INTEGER NOT NULL,
                    month TEXT NOT NULL,
                    count INTEGER NOT NULL,
                    PRIMARY KEY(api_key_id, month),
                    FOREIGN KEY(api_key_id) REFERENCES api_keys(id)
                );
                ",
            )?;
            Ok::<(), rusqlite::Error>(())
        })
        .await
        .context("Failed to initialize auth db schema")?;
        Ok(())
    }

    pub async fn upsert_user(
        &self,
        osm_uid: i64,
        display_name: &str,
        defaults: UserDefaults,
    ) -> Result<UserRecord> {
        let created_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let display_name = display_name.to_string();
        let conn = self.conn.clone();
        conn.call(move |conn| {
            let _ = conn.execute(
                "
                INSERT INTO users (osm_uid, display_name, created_at, daily_limit, monthly_limit)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(osm_uid) DO UPDATE SET display_name = excluded.display_name
                ",
                params![
                    osm_uid,
                    display_name,
                    created_at,
                    defaults.daily_limit,
                    defaults.monthly_limit
                ],
            );

            let mut stmt = conn.prepare(
                "SELECT id, osm_uid, display_name, daily_limit, monthly_limit
                 FROM users WHERE osm_uid = ?1",
            )?;
            let record = stmt.query_row(params![osm_uid], |row| {
                Ok(UserRecord {
                    id: row.get(0)?,
                    osm_uid: row.get(1)?,
                    display_name: row.get(2)?,
                    daily_limit: row.get(3)?,
                    monthly_limit: row.get(4)?,
                })
            })?;
            Ok::<UserRecord, rusqlite::Error>(record)
        })
        .await
        .context("Failed to upsert user")
    }

    pub async fn revoke_user_keys(&self, user_id: i64) -> Result<()> {
        let revoked_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let conn = self.conn.clone();
        conn.call(move |conn| {
            conn.execute(
                "UPDATE api_keys SET revoked_at = ?1 WHERE user_id = ?2 AND revoked_at IS NULL",
                params![revoked_at, user_id],
            )?;
            Ok::<(), rusqlite::Error>(())
        })
        .await
        .context("Failed to revoke user keys")?;
        Ok(())
    }

    pub async fn insert_api_key(
        &self,
        user_id: i64,
        key_prefix: &str,
        key_hash: &str,
    ) -> Result<i64> {
        let created_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let key_prefix = key_prefix.to_string();
        let key_hash = key_hash.to_string();
        let conn = self.conn.clone();
        conn.call(move |conn| {
            conn.execute(
                "INSERT INTO api_keys (user_id, key_prefix, key_hash, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![user_id, key_prefix, key_hash, created_at],
            )?;
            Ok::<i64, rusqlite::Error>(conn.last_insert_rowid())
        })
        .await
        .context("Failed to insert API key")
    }

    pub async fn lookup_key_by_hash(&self, key_hash: &str) -> Result<Option<AuthenticatedKey>> {
        let key_hash = key_hash.to_string();
        let conn = self.conn.clone();
        conn.call(move |conn| {
            let mut stmt = conn.prepare(
                "
                SELECT api_keys.id,
                       api_keys.user_id,
                       users.osm_uid,
                       users.display_name,
                       api_keys.key_prefix,
                       api_keys.created_at,
                       users.daily_limit,
                       users.monthly_limit
                  FROM api_keys
                  JOIN users ON api_keys.user_id = users.id
                 WHERE api_keys.key_hash = ?1
                   AND api_keys.revoked_at IS NULL
                ",
            )?;
            let record = stmt
                .query_row(params![key_hash], |row| {
                    Ok(AuthenticatedKey {
                        api_key_id: row.get(0)?,
                        user_id: row.get(1)?,
                        osm_uid: row.get(2)?,
                        display_name: row.get(3)?,
                        key_prefix: row.get(4)?,
                        created_at: row.get(5)?,
                        daily_limit: row.get(6)?,
                        monthly_limit: row.get(7)?,
                    })
                })
                .optional()?;
            Ok::<Option<AuthenticatedKey>, rusqlite::Error>(record)
        })
        .await
        .context("Failed to lookup API key")
    }

    pub async fn usage_for_user(
        &self,
        user_id: i64,
        day: &str,
        month: &str,
    ) -> Result<UsageSnapshot> {
        let day = day.to_string();
        let month = month.to_string();
        let conn = self.conn.clone();
        conn.call(move |conn| {
            let daily: i64 = conn.query_row(
                "
                SELECT COALESCE(SUM(usage_daily.count), 0)
                  FROM usage_daily
                  JOIN api_keys ON usage_daily.api_key_id = api_keys.id
                 WHERE api_keys.user_id = ?1
                   AND usage_daily.day = ?2
                ",
                params![user_id, day],
                |row| row.get(0),
            )?;
            let monthly: i64 = conn.query_row(
                "
                SELECT COALESCE(SUM(usage_monthly.count), 0)
                  FROM usage_monthly
                  JOIN api_keys ON usage_monthly.api_key_id = api_keys.id
                 WHERE api_keys.user_id = ?1
                   AND usage_monthly.month = ?2
                ",
                params![user_id, month],
                |row| row.get(0),
            )?;
            Ok::<UsageSnapshot, rusqlite::Error>(UsageSnapshot {
                daily_count: daily,
                monthly_count: monthly,
            })
        })
        .await
        .context("Failed to load usage snapshot")
    }

    pub async fn increment_usage(&self, api_key_id: i64, day: &str, month: &str) -> Result<()> {
        let day = day.to_string();
        let month = month.to_string();
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let conn = self.conn.clone();
        conn.call(move |conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "
                INSERT INTO usage_daily (api_key_id, day, count)
                VALUES (?1, ?2, 1)
                ON CONFLICT(api_key_id, day) DO UPDATE SET count = count + 1
                ",
                params![api_key_id, day],
            )?;
            tx.execute(
                "
                INSERT INTO usage_monthly (api_key_id, month, count)
                VALUES (?1, ?2, 1)
                ON CONFLICT(api_key_id, month) DO UPDATE SET count = count + 1
                ",
                params![api_key_id, month],
            )?;
            tx.execute(
                "UPDATE api_keys SET last_used_at = ?1 WHERE id = ?2",
                params![now, api_key_id],
            )?;
            tx.commit()?;
            Ok::<(), rusqlite::Error>(())
        })
        .await
        .context("Failed to increment usage")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_user_and_key_flow() {
        let db = AuthDb::new(Path::new(":memory:")).await.expect("db");
        let user = db
            .upsert_user(
                123,
                "tester",
                UserDefaults {
                    daily_limit: Some(10),
                    monthly_limit: None,
                },
            )
            .await
            .expect("user");
        assert_eq!(user.osm_uid, 123);
        assert_eq!(user.daily_limit, Some(10));

        db.revoke_user_keys(user.id).await.expect("revoke");
        let key_id = db
            .insert_api_key(user.id, "osmcs_1234", "hash")
            .await
            .expect("insert");
        let key = db.lookup_key_by_hash("hash").await.expect("lookup");
        let key = key.expect("key");
        assert_eq!(key.api_key_id, key_id);
        assert_eq!(key.display_name, "tester");
    }

    #[tokio::test]
    async fn test_usage_tracking() {
        let db = AuthDb::new(Path::new(":memory:")).await.expect("db");
        let user = db
            .upsert_user(
                321,
                "usage",
                UserDefaults {
                    daily_limit: None,
                    monthly_limit: Some(100),
                },
            )
            .await
            .expect("user");
        let key_id = db
            .insert_api_key(user.id, "osmcs_5678", "hash2")
            .await
            .expect("insert");

        db.increment_usage(key_id, "20250101", "202501")
            .await
            .expect("usage");
        db.increment_usage(key_id, "20250101", "202501")
            .await
            .expect("usage");

        let usage = db
            .usage_for_user(user.id, "20250101", "202501")
            .await
            .expect("snapshot");
        assert_eq!(usage.daily_count, 2);
        assert_eq!(usage.monthly_count, 2);
    }
}
