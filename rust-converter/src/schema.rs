use arrow::datatypes::{DataType, Field, Fields, Schema, TimeUnit};
use std::sync::Arc;

pub fn create_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("replication_sequence", DataType::UInt64, true),
        Field::new(
            "created_at",
            DataType::Timestamp(TimeUnit::Millisecond, None),
            true,
        ),
        Field::new(
            "closed_at",
            DataType::Timestamp(TimeUnit::Millisecond, None),
            true,
        ),
        Field::new("open", DataType::Boolean, false),
        Field::new("user", DataType::Utf8, true),
        Field::new("uid", DataType::Int64, true),
        Field::new("min_lat", DataType::Float64, true),
        Field::new("min_lon", DataType::Float64, true),
        Field::new("max_lat", DataType::Float64, true),
        Field::new("max_lon", DataType::Float64, true),
        Field::new("num_changes", DataType::UInt32, false),
        Field::new("comments_count", DataType::UInt32, false),
        Field::new("description", DataType::Utf8, true),
        Field::new(
            "tags",
            DataType::Map(
                Arc::new(Field::new(
                    "entries",
                    DataType::Struct(Fields::from(vec![
                        Field::new("keys", DataType::Utf8, false),
                        Field::new("values", DataType::Utf8, true),
                    ])),
                    false,
                )),
                false,
            ),
            false,
        ),
    ]))
}
