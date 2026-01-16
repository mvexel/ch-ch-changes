use crate::auth::hash_key;
use crate::models::ApiError;
use crate::state::AppState;
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, Uri};
use axum::middleware::Next;
use axum::response::Response;
use chrono::Utc;

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    if req.method() == Method::OPTIONS {
        return Ok(next.run(req).await);
    }

    let path = req.uri().path();
    if is_public_path(path) {
        return Ok(next.run(req).await);
    }

    let raw_key = extract_api_key(req.headers(), req.uri()).ok_or_else(|| {
        ApiError::unauthorized(
            "Missing API key. Use Authorization: Bearer <key> or X-API-Key header.",
        )
    })?;

    let key_hash = hash_key(&state.auth.config.key_secret, &raw_key);
    let auth_key = state
        .auth
        .db
        .lookup_key_by_hash(&key_hash)
        .await
        .map_err(|err| {
            tracing::error!("Auth DB lookup failed: {}", err);
            ApiError::internal_error("Failed to validate API key")
        })?
        .ok_or_else(|| ApiError::unauthorized("Invalid API key"))?;

    let now = Utc::now();
    let day = now.format("%Y%m%d").to_string();
    let month = now.format("%Y%m").to_string();

    let usage = state
        .auth
        .db
        .usage_for_user(auth_key.user_id, &day, &month)
        .await
        .map_err(|err| {
            tracing::error!("Usage lookup failed: {}", err);
            ApiError::internal_error("Failed to validate API key usage")
        })?;

    if let Some(limit) = auth_key.daily_limit {
        if usage.daily_count >= limit {
            return Err(ApiError::too_many_requests(
                "Daily request limit exceeded for this account",
            ));
        }
    }
    if let Some(limit) = auth_key.monthly_limit {
        if usage.monthly_count >= limit {
            return Err(ApiError::too_many_requests(
                "Monthly request limit exceeded for this account",
            ));
        }
    }

    req.extensions_mut().insert(auth_key.clone());
    let mut response = next.run(req).await;

    let daily_remaining = auth_key
        .daily_limit
        .map(|limit| limit.saturating_sub(usage.daily_count + 1));
    let monthly_remaining = auth_key
        .monthly_limit
        .map(|limit| limit.saturating_sub(usage.monthly_count + 1));
    attach_rate_limit_headers(
        &mut response,
        auth_key.daily_limit,
        daily_remaining,
        auth_key.monthly_limit,
        monthly_remaining,
    );

    if let Err(err) = state
        .auth
        .db
        .increment_usage(auth_key.api_key_id, &day, &month)
        .await
    {
        tracing::error!(
            "Failed to record usage for key {}: {}",
            auth_key.key_prefix,
            err
        );
    }

    Ok(response)
}

fn is_public_path(path: &str) -> bool {
    matches!(path, "/health" | "/meta/latest") || path.starts_with("/auth/osm/")
}

fn extract_api_key(headers: &HeaderMap, uri: &Uri) -> Option<String> {
    if let Some(value) = headers.get(header::AUTHORIZATION) {
        if let Ok(value) = value.to_str() {
            let mut parts = value.splitn(2, ' ');
            let scheme = parts.next()?.trim();
            let token = parts.next()?.trim();
            if scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }

    if let Some(value) = headers.get("x-api-key") {
        if let Ok(value) = value.to_str() {
            let token = value.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }

    if let Some(query) = uri.query() {
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            if key == "api_key" && !value.trim().is_empty() {
                return Some(value.into_owned());
            }
        }
    }

    None
}

fn attach_rate_limit_headers(
    response: &mut Response,
    daily_limit: Option<i64>,
    daily_remaining: Option<i64>,
    monthly_limit: Option<i64>,
    monthly_remaining: Option<i64>,
) {
    let headers = response.headers_mut();
    if let Some(limit) = daily_limit {
        if let Ok(value) = HeaderValue::from_str(&limit.to_string()) {
            headers.insert("X-RateLimit-Limit-Daily", value);
        }
    }
    if let Some(remaining) = daily_remaining {
        if let Ok(value) = HeaderValue::from_str(&remaining.to_string()) {
            headers.insert("X-RateLimit-Remaining-Daily", value);
        }
    }
    if let Some(limit) = monthly_limit {
        if let Ok(value) = HeaderValue::from_str(&limit.to_string()) {
            headers.insert("X-RateLimit-Limit-Monthly", value);
        }
    }
    if let Some(remaining) = monthly_remaining {
        if let Ok(value) = HeaderValue::from_str(&remaining.to_string()) {
            headers.insert("X-RateLimit-Remaining-Monthly", value);
        }
    }
}
