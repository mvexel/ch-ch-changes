//! Response formatters
//!
//! Implements formatters for different response formats (XML, JSON, JSONL).

pub mod json;
pub mod jsonl;
pub mod xml;

use crate::models::Changeset;

/// Trait for formatting changesets into various output formats
///
/// # TODO
/// - Implement format trait
/// - Add streaming formatter support
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

/// Format type enumeration
#[derive(Debug, Clone, Copy)]
pub enum FormatType {
    Xml,
    Json,
    JsonLines,
}

impl FormatType {
    /// Parse format from string
    ///
    /// # TODO
    /// - Add case-insensitive matching
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "xml" => Some(FormatType::Xml),
            "json" => Some(FormatType::Json),
            "jsonl" | "jsonlines" => Some(FormatType::JsonLines),
            _ => None,
        }
    }
}
