//! Response formatters
//!
//! Implements formatters for different response formats (XML, JSON, JSONL).

pub mod json;
pub mod jsonl;
pub mod xml;

use crate::models::Changeset;

/// Trait for formatting changesets into various output formats
pub trait Formatter {
    /// Format a single changeset
    fn format_changeset(&self, changeset: &Changeset)
        -> Result<String, Box<dyn std::error::Error>>;

    /// Format multiple changesets
    fn format_changesets(
        &self,
        changesets: &[Changeset],
    ) -> Result<String, Box<dyn std::error::Error>>;
}

/// Trait for streaming formatters that can emit data incrementally
pub trait StreamingFormatter: Send + Sync {
    /// Returns the content type for HTTP responses
    fn content_type(&self) -> &'static str;

    /// Returns the header to emit before any items (e.g., XML declaration, JSON opening)
    fn stream_header(&self) -> String;

    /// Format a single item for streaming output
    /// `is_first` indicates if this is the first item (useful for comma handling in JSON arrays)
    fn stream_item(&self, changeset: &Changeset, is_first: bool) -> Result<String, Box<dyn std::error::Error + Send + Sync>>;

    /// Returns the footer to emit after all items (e.g., closing tags)
    fn stream_footer(&self) -> String;
}

/// Format type enumeration
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum FormatType {
    Xml,
    Json,
    JsonLines,
}

impl FormatType {
    /// Parse format from string
    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "xml" => Some(FormatType::Xml),
            "json" => Some(FormatType::Json),
            "jsonl" | "jsonlines" => Some(FormatType::JsonLines),
            _ => None,
        }
    }
}
