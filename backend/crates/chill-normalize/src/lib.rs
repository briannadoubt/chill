//! Canonical decoding, timing classification, and durable inbox processing.

mod decode;
mod processor;
mod timing;
mod types;
mod validator;

pub use decode::Decoder;
pub use processor::{Processor, ProcessorConfiguration, ProcessorError};
pub use timing::{Timing, TimingClass, TimingPolicy};
pub use types::{InboxItem, Record};
