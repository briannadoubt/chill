//! Deterministic Parquet encoding and immutable lake manifests.

mod manifest;
mod model;
mod parquet;
mod processor;

pub use manifest::encode_manifest;
pub use model::{
    Batch, DATASET_SCHEMA_VERSION, LakeError, Manifest, Published, Row, WRITER_FORMAT_VERSION,
    deterministic_batch_id,
};
pub use parquet::{decode_parquet, encode_parquet};
pub use processor::{Processor, ProcessorConfiguration, ProcessorError};
