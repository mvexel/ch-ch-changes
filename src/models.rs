use crate::Changeset;
use chrono::{Datelike, TimeZone, Utc};
use std::collections::HashMap;

/// Partition key for year/month partitioning
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct PartitionKey {
    pub year: i32,
    pub month: u32,
}

impl PartitionKey {
    pub fn from_timestamp_millis(ts: i64) -> Self {
        let dt = Utc.timestamp_millis_opt(ts).unwrap();
        Self {
            year: dt.year(),
            month: dt.month(),
        }
    }

    pub fn dir_path(&self) -> String {
        format!("year={}/month={:02}", self.year, self.month)
    }

    pub fn legacy_file_path(&self) -> String {
        format!("year={}/month={:02}.parquet", self.year, self.month)
    }
}

#[derive(Debug, Clone, Default)]
pub struct PartitionStats {
    pub changeset_count: u64,
    pub min_id: i64,
    pub max_id: i64,
    pub min_timestamp: Option<i64>,
    pub max_timestamp: Option<i64>,
}

impl PartitionStats {
    pub fn from_changeset(cs: &Changeset) -> Self {
        Self {
            changeset_count: 1,
            min_id: cs.id,
            max_id: cs.id,
            min_timestamp: cs.created_at,
            max_timestamp: cs.created_at,
        }
    }

    pub fn update(&mut self, cs: &Changeset) {
        self.changeset_count = self.changeset_count.saturating_add(1);
        self.min_id = self.min_id.min(cs.id);
        self.max_id = self.max_id.max(cs.id);

        if let Some(ts) = cs.created_at {
            self.min_timestamp = match self.min_timestamp {
                Some(current) => Some(current.min(ts)),
                None => Some(ts),
            };
            self.max_timestamp = match self.max_timestamp {
                Some(current) => Some(current.max(ts)),
                None => Some(ts),
            };
        }
    }

    pub fn merge(&mut self, other: &PartitionStats) {
        self.changeset_count += other.changeset_count;
        self.min_id = self.min_id.min(other.min_id);
        self.max_id = self.max_id.max(other.max_id);
        
        if let Some(ts) = other.min_timestamp {
            self.min_timestamp = match self.min_timestamp {
                Some(current) => Some(current.min(ts)),
                None => Some(ts),
            };
        }
        if let Some(ts) = other.max_timestamp {
            self.max_timestamp = match self.max_timestamp {
                Some(current) => Some(current.max(ts)),
                None => Some(ts),
            };
        }
    }
}

pub struct PartitionSummary {
    pub counts: HashMap<PartitionKey, usize>,
    pub stats: HashMap<PartitionKey, PartitionStats>,
}

impl PartitionSummary {
    pub fn merge(&mut self, other: PartitionSummary) {
        for (key, count) in other.counts {
            *self.counts.entry(key.clone()).or_insert(0) += count;
        }
        for (key, stats) in other.stats {
            self.stats.entry(key).or_default().merge(&stats);
        }
    }
}

/// Tracks user statistics for index generation
#[derive(Debug, Default)]
pub struct UserStats {
    pub uid: i64,
    pub username: Option<String>,
    pub changeset_count: u64,
    pub first_changeset_at: Option<i64>,
    pub last_changeset_at: Option<i64>,
}
