//! OpenAPI specification module
//!
//! Builds the OpenAPI/Swagger specification for the API.

use crate::routes::{changesets, meta};
use crate::models::Changeset;
use crate::models::error::{ErrorResponse, ErrorDetail};
use crate::routes::meta::{HealthResponse, ReplicationStateResponse};
use utoipa::OpenApi;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};

/// OpenAPI specification
#[derive(OpenApi)]
#[openapi(
    paths(
        // Meta endpoints
        meta::health,
        meta::get_replication_state,

        // Changeset endpoints
        changesets::get_changesets_by_day,
    ),
    components(
        schemas(
            Changeset,
            HealthResponse,
            ReplicationStateResponse,
            ErrorResponse,
            ErrorDetail,
        )
    ),
    tags(
        (name = "Meta", description = "Health and metadata endpoints"),
        (name = "Changesets", description = "Changeset query endpoints")
    ),
    info(
        title = "OSM Changeset Historic API",
        version = env!("CARGO_PKG_VERSION"),
        description = "API for querying OpenStreetMap changeset metadata by date, time, or ID range.",
        license(
            name = "MIT",
        ),
        contact(
            name = "API Support",
            url = "https://codeberg.org/mvexel/osm-changeset-api"
        )
    ),
    servers(
        (url = "https://chch.osm.lol", description = "Production server"),
        (url = "http://localhost:3000", description = "Local development server")
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

/// Security scheme addon
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "api_key",
                SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("Authorization"))),
            )
        }
    }
}
