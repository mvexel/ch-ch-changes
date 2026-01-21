use crate::auth::db::UserDefaults;
use crate::API_VERSION;
use anyhow::{Context, Result};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub mod config;
pub mod db;
pub mod middleware;
pub mod routes;

pub use config::AuthConfig;
pub use db::{AuthDb, AuthenticatedKey};

const OAUTH_STATE_TTL: Duration = Duration::from_secs(600);
const API_KEY_PREFIX: &str = "osmcs_";

#[derive(Clone)]
pub struct AuthState {
    pub db: AuthDb,
    pub config: AuthConfig,
    pub http_client: reqwest::Client,
    oauth_states: Arc<Mutex<HashMap<String, Instant>>>,
}

#[derive(Debug)]
pub struct IssuedKey {
    pub key: String,
    pub prefix: String,
}

impl AuthState {
    pub async fn new(config: AuthConfig) -> Result<Self> {
        let db = AuthDb::new(&config.db_path).await?;
        let http_client = reqwest::Client::builder()
            .user_agent(format!("changeset-server/{}", API_VERSION))
            .build()
            .context("Failed to create OAuth HTTP client")?;
        Ok(Self {
            db,
            config,
            http_client,
            oauth_states: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn issue_state(&self) -> String {
        let state = generate_token(24);
        let mut guard = self.oauth_states.lock().await;
        let now = Instant::now();
        guard.retain(|_, created| now.duration_since(*created) < OAUTH_STATE_TTL);
        guard.insert(state.clone(), now);
        state
    }

    pub async fn consume_state(&self, state: &str) -> bool {
        let mut guard = self.oauth_states.lock().await;
        if let Some(created) = guard.remove(state) {
            return Instant::now().duration_since(created) < OAUTH_STATE_TTL;
        }
        false
    }

    pub fn default_limits(&self) -> UserDefaults {
        UserDefaults {
            daily_limit: self.config.default_daily_limit,
            monthly_limit: self.config.default_monthly_limit,
        }
    }

    pub async fn issue_api_key(&self, user_id: i64) -> Result<IssuedKey> {
        self.db
            .revoke_user_keys(user_id)
            .await
            .context("Failed to revoke existing keys")?;
        let key = generate_api_key();
        let prefix = key.chars().take(8).collect::<String>();
        let key_secret = self.config.key_secret.as_ref()
            .ok_or_else(|| anyhow::anyhow!("AUTH_KEY_SECRET not configured"))?;
        let key_hash = hash_key(key_secret, &key);
        self.db
            .insert_api_key(user_id, &prefix, &key_hash)
            .await
            .context("Failed to store API key")?;
        Ok(IssuedKey { key, prefix })
    }
}

pub fn hash_key(secret: &str, key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b":");
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

fn generate_api_key() -> String {
    format!("{}{}", API_KEY_PREFIX, generate_token(32))
}

fn generate_token(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut buffer);
    hex::encode(buffer)
}
