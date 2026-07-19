#![no_main]

use std::time::Duration;

use chill_query::Plan;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(plan) = serde_json::from_slice::<Plan>(data) {
        let _ = plan.validate(Duration::from_secs(31 * 24 * 60 * 60), 10_000);
    }
});
