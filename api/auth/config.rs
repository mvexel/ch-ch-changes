use anyhow::{anyhow, Result};
use std::env;
use std::path::PathBuf;

#[derive(Clone)]
pub struct AuthConfig {
    pub oauth_client_id: String,
    pub oauth_client_secret: String,
    pub oauth_redirect_url: String,
    pub oauth_authorize_url: String,
    pub oauth_token_url: String,
    pub oauth_user_details_url: String,
    pub oauth_scope: String,
    pub key_secret: String,
    pub db_path: PathBuf,
    pub default_daily_limit: Option<i64>,
    pub default_monthly_limit: Option<i64>,
}

impl AuthConfig {
    pub fn from_env() -> Result<Self> {
        let oauth_client_id = require_env("OSM_OAUTH_CLIENT_ID")?;
        let oauth_client_secret = require_env("OSM_OAUTH_CLIENT_SECRET")?;
        let oauth_redirect_url = require_env("OSM_OAUTH_REDIRECT_URL")?;
        let key_secret = require_env("AUTH_KEY_SECRET")?;
        if key_secret.trim().is_empty() {
            return Err(anyhow!("AUTH_KEY_SECRET is required but empty"));
        }

        let oauth_authorize_url = env::var("OSM_OAUTH_AUTHORIZE_URL")
            .unwrap_or_else(|_| "https://www.openstreetmap.org/oauth2/authorize".to_string());
        let oauth_token_url = env::var("OSM_OAUTH_TOKEN_URL")
            .unwrap_or_else(|_| "https://www.openstreetmap.org/oauth2/token".to_string());
        let oauth_user_details_url = env::var("OSM_OAUTH_USER_DETAILS_URL")
            .unwrap_or_else(|_| "https://api.openstreetmap.org/api/0.6/user/details.json".to_string());
        let oauth_scope =
            env::var("OSM_OAUTH_SCOPE").unwrap_or_else(|_| "read_prefs".to_string());

        let default_daily_limit =
            parse_limit(env::var("AUTH_DEFAULT_DAILY_LIMIT").ok()).map_err(|err| {
                anyhow!("AUTH_DEFAULT_DAILY_LIMIT {}", err)
            })?;
        let default_monthly_limit =
            parse_limit(env::var("AUTH_DEFAULT_MONTHLY_LIMIT").ok()).map_err(|err| {
                anyhow!("AUTH_DEFAULT_MONTHLY_LIMIT {}", err)
            })?;

        Ok(Self {
            oauth_client_id,
            oauth_client_secret,
            oauth_redirect_url,
            oauth_authorize_url,
            oauth_token_url,
            oauth_user_details_url,
            oauth_scope,
            key_secret,
            db_path: resolve_db_path(),
            default_daily_limit,
            default_monthly_limit,
        })
    }
}

fn require_env(key: &str) -> Result<String> {
    env::var(key).map_err(|_| anyhow!("{} is required but not set", key))
}

fn parse_limit(value: Option<String>) -> Result<Option<i64>, &'static str> {
    match value {
        Some(raw) => {
            let parsed = raw
                .trim()
                .parse::<i64>()
                .map_err(|_| "must be an integer")?;
            if parsed <= 0 {
                Ok(None)
            } else {
                Ok(Some(parsed))
            }
        }
        None => Ok(None),
    }
}

fn resolve_db_path() -> PathBuf {
    if let Ok(path) = env::var("AUTH_DB_PATH") {
        return PathBuf::from(path);
    }
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home).join("changeset-auth.sqlite");
    }
    PathBuf::from("./changeset-auth.sqlite")
}
