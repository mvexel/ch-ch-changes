use crate::models::ApiError;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Redirect, Response};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct OAuthCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct OsmUserResponse {
    user: OsmUser,
}

#[derive(Debug, Deserialize)]
struct OsmUser {
    id: i64,
    display_name: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct MeResponse {
    /// OSM user ID
    pub osm_uid: i64,
    /// OSM display name
    pub display_name: String,
    /// API key prefix for identification
    pub key_prefix: String,
    /// Timestamp when the API key was created
    pub key_created_at: String,
    /// Daily request limit (null if unlimited)
    pub daily_limit: Option<i64>,
    /// Monthly request limit (null if unlimited)
    pub monthly_limit: Option<i64>,
    /// Number of requests made today
    pub daily_usage: i64,
    /// Number of requests made this month
    pub monthly_usage: i64,
}

pub async fn oauth_start(State(state): State<AppState>) -> Result<Redirect, ApiError> {
    let client_id = state.auth.config.oauth_client_id.as_ref().ok_or_else(|| {
        ApiError::internal_error("OAuth not configured")
    })?;
    let redirect_url = state.auth.config.oauth_redirect_url.as_ref().ok_or_else(|| {
        ApiError::internal_error("OAuth not configured")
    })?;

    let oauth_state = state.auth.issue_state().await;
    let mut url = reqwest::Url::parse(&state.auth.config.oauth_authorize_url).map_err(|e| {
        tracing::error!("Invalid OSM OAuth authorize URL: {}", e);
        ApiError::internal_error("OAuth configuration error")
    })?;

    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_url)
        .append_pair("response_type", "code")
        .append_pair("scope", &state.auth.config.oauth_scope)
        .append_pair("state", &oauth_state);

    Ok(Redirect::temporary(url.as_str()))
}

pub async fn oauth_callback(
    State(state): State<AppState>,
    Query(params): Query<OAuthCallbackQuery>,
) -> Result<Response, ApiError> {
    if let Some(error) = params.error {
        let details = params
            .error_description
            .unwrap_or_else(|| "OAuth authorization failed".to_string());
        return Err(ApiError::bad_request_with_details(error, details));
    }

    let code = params
        .code
        .ok_or_else(|| ApiError::bad_request("Missing OAuth code"))?;
    let state_param = params
        .state
        .ok_or_else(|| ApiError::bad_request("Missing OAuth state"))?;

    if !state.auth.consume_state(&state_param).await {
        return Err(ApiError::bad_request("Invalid OAuth state"));
    }

    let token = exchange_code(&state, &code).await?;
    let user = fetch_user(&state, &token).await?;

    let user_record = state
        .auth
        .db
        .upsert_user(user.id, &user.display_name, state.auth.default_limits())
        .await
        .map_err(|err| {
            tracing::error!("Failed to upsert user: {}", err);
            ApiError::internal_error("Failed to store user record")
        })?;

    let issued_key = state
        .auth
        .issue_api_key(user_record.id)
        .await
        .map_err(|err| {
            tracing::error!("Failed to issue API key: {}", err);
            ApiError::internal_error("Failed to issue API key")
        })?;

    tracing::info!(
        "Issued API key {} for osm_uid {}",
        issued_key.prefix,
        user_record.osm_uid
    );

    let body = render_key_page(
        &user_record.display_name,
        &issued_key.key,
        user_record.daily_limit,
        user_record.monthly_limit,
    );
    Ok(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response())
}

/// Get current user information and API usage
#[utoipa::path(
    get,
    path = "/auth/me",
    responses(
        (status = 200, description = "Current user information and API usage", body = MeResponse),
        (status = 401, description = "Unauthorized - invalid or missing API key", body = crate::models::error::ErrorResponse),
        (status = 500, description = "Internal server error", body = crate::models::error::ErrorResponse)
    ),
    tag = "Auth",
    security(
        ("api_key" = [])
    )
)]
pub async fn get_me(
    State(state): State<AppState>,
    axum::extract::Extension(auth_key): axum::extract::Extension<crate::auth::AuthenticatedKey>,
) -> Result<axum::Json<MeResponse>, ApiError> {
    let now = chrono::Utc::now();
    let day = now.format("%Y%m%d").to_string();
    let month = now.format("%Y%m").to_string();

    let usage = state
        .auth
        .db
        .usage_for_user(auth_key.user_id, &day, &month)
        .await
        .map_err(|err| {
            tracing::error!(
                "Failed to load usage for user {}: {}",
                auth_key.user_id,
                err
            );
            ApiError::internal_error("Failed to load usage")
        })?;

    Ok(axum::Json(MeResponse {
        osm_uid: auth_key.osm_uid,
        display_name: auth_key.display_name,
        key_prefix: auth_key.key_prefix,
        key_created_at: auth_key.created_at,
        daily_limit: auth_key.daily_limit,
        monthly_limit: auth_key.monthly_limit,
        daily_usage: usage.daily_count,
        monthly_usage: usage.monthly_count,
    }))
}

async fn exchange_code(state: &AppState, code: &str) -> Result<String, ApiError> {
    let client_id = state.auth.config.oauth_client_id.as_ref().ok_or_else(|| {
        ApiError::internal_error("OAuth not configured")
    })?;
    let client_secret = state.auth.config.oauth_client_secret.as_ref().ok_or_else(|| {
        ApiError::internal_error("OAuth not configured")
    })?;
    let redirect_url = state.auth.config.oauth_redirect_url.as_ref().ok_or_else(|| {
        ApiError::internal_error("OAuth not configured")
    })?;

    let client = state.auth.http_client.clone();
    let response = client
        .post(&state.auth.config.oauth_token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_url),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ])
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|err| {
            tracing::error!("OAuth token exchange failed: {}", err);
            ApiError::internal_error("OAuth token exchange failed")
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::error!("OAuth token exchange error {}: {}", status, body);
        return Err(ApiError::internal_error("OAuth token exchange failed"));
    }

    let token: TokenResponse = response.json().await.map_err(|err| {
        tracing::error!("OAuth token parse failed: {}", err);
        ApiError::internal_error("OAuth token exchange failed")
    })?;

    Ok(token.access_token)
}

async fn fetch_user(state: &AppState, access_token: &str) -> Result<OsmUser, ApiError> {
    let client = state.auth.http_client.clone();
    let response = client
        .get(&state.auth.config.oauth_user_details_url)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|err| {
            tracing::error!("OSM user request failed: {}", err);
            ApiError::internal_error("Failed to fetch OSM user details")
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::error!("OSM user request error {}: {}", status, body);
        return Err(ApiError::internal_error("Failed to fetch OSM user details"));
    }

    let payload: OsmUserResponse = response.json().await.map_err(|err| {
        tracing::error!("OSM user parse failed: {}", err);
        ApiError::internal_error("Failed to parse OSM user details")
    })?;

    Ok(payload.user)
}

fn render_key_page(
    display_name: &str,
    api_key: &str,
    daily_limit: Option<i64>,
    monthly_limit: Option<i64>,
) -> String {
    let display_name = escape_html(display_name);
    let daily_text = daily_limit
        .map(|limit| format!("Daily limit: {} requests", limit))
        .unwrap_or_else(|| "Daily limit: unlimited".to_string());
    let monthly_text = monthly_limit
        .map(|limit| format!("Monthly limit: {} requests", limit))
        .unwrap_or_else(|| "Monthly limit: unlimited".to_string());

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>OSM Changeset API Key</title>
    <style>
        body {{
            font-family: system-ui, -apple-system, sans-serif;
            max-width: 720px;
            margin: 40px auto;
            padding: 24px;
            line-height: 1.5;
            color: #222;
        }}
        .key {{
            font-family: "Courier New", monospace;
            background: #f4f4f4;
            padding: 12px;
            border-radius: 6px;
            word-break: break-all;
        }}
        .note {{
            margin-top: 16px;
            color: #555;
        }}
        a {{ color: #0066cc; }}
    </style>
</head>
<body>
    <h1>API key issued</h1>
    <p>Hi {display_name}, here is your new API key. Save it now. It will not be shown again.</p>
    <div class="key">{api_key}</div>
    <p class="note">{daily_text} | {monthly_text}</p>
    <h2>Use it in requests</h2>
    <pre><code>curl -H "Authorization: Bearer {api_key}" https://chch.osm.lol/changeset/1.json</code></pre>
    <p class="note">
        Logging in again will revoke the previous key and issue a new one.
    </p>
    <p><a href="/">Back to docs</a></p>
</body>
</html>"#
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
