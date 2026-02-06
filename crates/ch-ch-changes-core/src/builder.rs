use crate::Changeset;
use anyhow::Result;
use arrow::array::{
    ArrayRef, BooleanBuilder, Float64Builder, Int64Builder, MapBuilder, StringBuilder,
    TimestampMillisecondBuilder, UInt32Builder, UInt64Builder,
};
use arrow::datatypes::Schema;
use arrow::record_batch::RecordBatch;
use std::sync::Arc;

pub struct BatchBuilders {
    id: Int64Builder,
    replication_sequence: UInt64Builder,
    created_at: TimestampMillisecondBuilder,
    closed_at: TimestampMillisecondBuilder,
    open: BooleanBuilder,
    user: StringBuilder,
    uid: Int64Builder,
    min_lat: Float64Builder,
    min_lon: Float64Builder,
    max_lat: Float64Builder,
    max_lon: Float64Builder,
    num_changes: UInt32Builder,
    comments_count: UInt32Builder,
    description: StringBuilder,
    tags: MapBuilder<StringBuilder, StringBuilder>,
    len: usize,
}

impl BatchBuilders {
    pub fn with_capacity(capacity: usize) -> Self {
        // For string data we start with a modest byte capacity and Arrow will grow as needed.
        let string_byte_capacity = capacity * 16;

        Self {
            id: Int64Builder::with_capacity(capacity),
            replication_sequence: UInt64Builder::with_capacity(capacity),
            created_at: TimestampMillisecondBuilder::with_capacity(capacity),
            closed_at: TimestampMillisecondBuilder::with_capacity(capacity),
            open: BooleanBuilder::with_capacity(capacity),
            user: StringBuilder::with_capacity(capacity, string_byte_capacity),
            uid: Int64Builder::with_capacity(capacity),
            min_lat: Float64Builder::with_capacity(capacity),
            min_lon: Float64Builder::with_capacity(capacity),
            max_lat: Float64Builder::with_capacity(capacity),
            max_lon: Float64Builder::with_capacity(capacity),
            num_changes: UInt32Builder::with_capacity(capacity),
            comments_count: UInt32Builder::with_capacity(capacity),
            description: StringBuilder::with_capacity(capacity, string_byte_capacity),
            tags: MapBuilder::with_capacity(
                None,
                StringBuilder::with_capacity(capacity * 2, string_byte_capacity),
                StringBuilder::with_capacity(capacity * 2, string_byte_capacity),
                capacity,
            ),
            len: 0,
        }
    }

    pub fn append(&mut self, cs: &Changeset) {
        self.id.append_value(cs.id);

        if let Some(seq) = cs.replication_sequence {
            self.replication_sequence.append_value(seq);
        } else {
            self.replication_sequence.append_null();
        }

        if let Some(ts) = cs.created_at {
            self.created_at.append_value(ts);
        } else {
            self.created_at.append_null();
        }

        if let Some(ts) = cs.closed_at {
            self.closed_at.append_value(ts);
        } else {
            self.closed_at.append_null();
        }

        self.open.append_value(cs.open);

        if let Some(ref user) = cs.user {
            self.user.append_value(user);
        } else {
            self.user.append_null();
        }

        if let Some(uid) = cs.uid {
            self.uid.append_value(uid);
        } else {
            self.uid.append_null();
        }

        if let Some(lat) = cs.min_lat {
            self.min_lat.append_value(lat);
        } else {
            self.min_lat.append_null();
        }

        if let Some(lon) = cs.min_lon {
            self.min_lon.append_value(lon);
        } else {
            self.min_lon.append_null();
        }

        if let Some(lat) = cs.max_lat {
            self.max_lat.append_value(lat);
        } else {
            self.max_lat.append_null();
        }

        if let Some(lon) = cs.max_lon {
            self.max_lon.append_value(lon);
        } else {
            self.max_lon.append_null();
        }

        self.num_changes.append_value(cs.num_changes);
        self.comments_count.append_value(cs.comments_count);

        if let Some(ref description) = cs.description {
            self.description.append_value(description);
        } else {
            self.description.append_null();
        }

        // Append tags
        if !cs.tags.is_empty() {
            for (k, v) in &cs.tags {
                self.tags.keys().append_value(k);
                self.tags.values().append_value(v);
            }
            self.tags.append(true).unwrap();
        } else {
            self.tags.append(true).unwrap();
        }

        self.len += 1;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn finish_batch(&mut self, schema: &Arc<Schema>) -> Result<RecordBatch> {
        let columns: Vec<ArrayRef> = vec![
            Arc::new(self.id.finish()),
            Arc::new(self.replication_sequence.finish()),
            Arc::new(self.created_at.finish()),
            Arc::new(self.closed_at.finish()),
            Arc::new(self.open.finish()),
            Arc::new(self.user.finish()),
            Arc::new(self.uid.finish()),
            Arc::new(self.min_lat.finish()),
            Arc::new(self.min_lon.finish()),
            Arc::new(self.max_lat.finish()),
            Arc::new(self.max_lon.finish()),
            Arc::new(self.num_changes.finish()),
            Arc::new(self.comments_count.finish()),
            Arc::new(self.description.finish()),
            Arc::new(self.tags.finish()),
        ];

        let batch = RecordBatch::try_new(schema.clone(), columns)?;
        self.len = 0;
        Ok(batch)
    }
}
