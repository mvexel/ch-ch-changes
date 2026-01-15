//! Error response models for the API
//!
//! Provides structured error responses with appropriate HTTP status codes.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

/// Structured error response returned by API endpoints
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

/// Details of an error
#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    pub code: u16,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

/// API error type that can be converted into an HTTP response
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    pub details: Option<String>,
}

#[allow(dead_code)]
impl ApiError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            details: None,
        }
    }

    pub fn bad_request_with_details(message: impl Into<String>, details: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            details: Some(details.into()),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
            details: None,
        }
    }

    pub fn internal_error(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
            details: None,
        }
    }

    pub fn internal_error_with_details(message: impl Into<String>, details: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
            details: Some(details.into()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorResponse {
            error: ErrorDetail {
                code: self.status.as_u16(),
                message: self.message,
                details: self.details,
            },
        };

        (self.status, Json(body)).into_response()
    }
}
