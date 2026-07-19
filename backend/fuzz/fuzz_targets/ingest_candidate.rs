#![no_main]

use chill_ingest::{
    Candidate, Limits, PayloadFormat, SignalKind, validate_candidate,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let selector = data.first().copied().unwrap_or_default();
    let kind = match selector % 3 {
        0 => SignalKind::Logs,
        1 => SignalKind::Traces,
        _ => SignalKind::Metrics,
    };
    let format = if selector & 0x80 == 0 {
        PayloadFormat::Protobuf
    } else {
        PayloadFormat::Json
    };
    let candidate = Candidate {
        kind,
        format,
        payload: data.get(1..).unwrap_or_default().to_vec(),
        credential: String::from("ch_sk_invalid"),
        idempotency_key: String::new(),
        replay: None,
    };
    let _ = validate_candidate(candidate, Limits::default());
});
