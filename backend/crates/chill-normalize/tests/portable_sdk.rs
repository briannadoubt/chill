//! Representative OTLP/HTTP JSON emitted by portable server and web SDKs.
#![allow(clippy::unwrap_used, reason = "test setup and assertions fail fast")]

use chill_normalize::{Decoder, InboxItem, TimingPolicy};
use chill_observability::{Client, Config, Consent, SemanticName, ServiceName};
use serde_json::json;
use std::{collections::BTreeMap, fs};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const PORTABLE_OTLP: &[u8] = include_bytes!("../../../testdata/portable-sdk-otlp.json");
const BEHAVIOR_SCHEMA: &str = include_str!("../../../../schemas/behavior/v1/envelope.schema.json");
const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

fn item(attempt_count: i32) -> InboxItem {
    item_with_payload(attempt_count, PORTABLE_OTLP.to_vec())
}

fn item_with_payload(attempt_count: i32, payload: Vec<u8>) -> InboxItem {
    InboxItem {
        id: 105,
        organization_id: "18e9a1b3-2a84-4b4e-90f5-d5039ae2eb60".to_owned(),
        project_id: "81be8b89-68bc-4fd6-a8d1-a956b8e568bb".to_owned(),
        environment_id: "8a0c2ddf-e5a5-4f55-bcaa-c5aa6fc2733c".to_owned(),
        data_source_id: "a7377206-3c43-464e-8dbb-682dcabde42d".to_owned(),
        signal_kind: "logs".to_owned(),
        payload_format: "json".to_owned(),
        payload,
        metadata: json!({"sdk":"portable-sdk"}),
        server_received_at: OffsetDateTime::UNIX_EPOCH + Duration::seconds(1_800_000_000),
        attempt_count,
    }
}

#[tokio::test]
async fn rust_sdk_generated_otlp_passes_the_production_normalizer() {
    let queue = std::env::temp_dir().join(format!("chill-normalize-sdk-{}", Uuid::new_v4()));
    let mut config = Config::new(
        ServiceName::try_from("portable.rust.normalizer").unwrap(),
        &queue,
        "http://127.0.0.1:4318/v1/logs",
    );
    config.consent = Consent::Granted;
    let client = Client::new(config).unwrap();
    client.start_session();
    client.event(
        SemanticName::try_from("portable.rust.event").unwrap(),
        BTreeMap::new(),
    );
    client.action(
        SemanticName::try_from("portable.rust.action").unwrap(),
        BTreeMap::new(),
    );
    let _: Result<(), ()> = client
        .activity(
            SemanticName::try_from("portable.rust.activity").unwrap(),
            BTreeMap::new(),
            async { Ok(()) },
        )
        .await;
    client.end_session();
    let records: Vec<_> = client
        .queue()
        .peek(16)
        .unwrap()
        .into_iter()
        .map(|(_, record)| record)
        .collect();
    let payload = serde_json::to_vec(&client.otlp_json(&records)).unwrap();

    let normalized = Decoder::new(TimingPolicy::default())
        .decode(&item_with_payload(1, payload))
        .unwrap();
    let schema: serde_json::Value = serde_json::from_str(BEHAVIOR_SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for record in &normalized {
        validator.validate(&record.canonical).unwrap();
        assert_eq!(record.canonical["privacy"]["policy_version"], "privacy-v1");
    }
    assert_eq!(normalized.len(), 6);
    assert_eq!(normalized[0].canonical["kind"], "session");
    assert_eq!(normalized[1].canonical["payload"]["event_class"], "custom");
    assert_eq!(normalized[2].canonical["payload"]["activation"], "system");
    assert_eq!(
        normalized[3].canonical["payload"]["activity_kind"],
        "custom"
    );
    assert_eq!(normalized[4].canonical["operation"], "end");
    assert_eq!(normalized[5].canonical["kind"], "session");
    assert_eq!(
        normalized[3].canonical["subject_id"],
        normalized[4].canonical["subject_id"]
    );
    assert_eq!(normalized[3].trace_id, normalized[4].trace_id);
    let serialized = normalized
        .iter()
        .map(|record| serde_json::to_string(&record.canonical).unwrap())
        .collect::<String>();
    assert!(!serialized.contains("CHILL_API_KEY"));
    let _ = fs::remove_dir_all(queue);
}

#[test]
fn portable_otlp_json_preserves_identity_and_correlation_without_canaries() {
    let records = Decoder::new(TimingPolicy::default())
        .decode(&item(1))
        .unwrap();
    let schema: serde_json::Value = serde_json::from_str(BEHAVIOR_SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for record in &records {
        validator.validate(&record.canonical).unwrap();
    }

    assert_eq!(records.len(), 2);
    assert_eq!(records[0].record_id, "0198a001-0000-7000-8000-000000000001");
    assert_eq!(records[1].record_id, "0198a001-0001-7000-8000-000000000002");
    assert_eq!(
        records[0].installation_id.as_deref(),
        Some("9f22f5c1-c67f-4c2d-9f04-0b1fc3000001")
    );
    assert_eq!(
        records[1].installation_id.as_deref(),
        Some("5e3e916f-8afd-43bd-9b43-81da6f000002")
    );
    assert_eq!(records[0].canonical["source"]["platform"], "server");
    assert_eq!(records[1].canonical["source"]["platform"], "web");
    assert_eq!(
        records[0].canonical["resource"]["attributes"]["telemetry.sdk.language"],
        "rust"
    );
    assert_eq!(
        records[1].canonical["resource"]["attributes"]["telemetry.sdk.language"],
        "javascript"
    );
    assert_eq!(records[0].trace_id.as_deref(), Some(TRACE_ID));
    assert_eq!(records[1].trace_id.as_deref(), Some(TRACE_ID));
    assert_ne!(records[0].span_id, records[1].span_id);

    let canonical = records
        .iter()
        .map(|record| serde_json::to_string(&record.canonical).unwrap())
        .collect::<String>();
    assert!(!canonical.contains("privacy-canary-server-4a8f"));
    assert!(!canonical.contains("privacy-canary-web-7d2c"));
}

#[test]
fn portable_otlp_retry_keeps_canonical_identity_stable() {
    let first = Decoder::new(TimingPolicy::default())
        .decode(&item(1))
        .unwrap();
    let retry = Decoder::new(TimingPolicy::default())
        .decode(&item(2))
        .unwrap();

    assert_eq!(first.len(), retry.len());
    for (before, after) in first.iter().zip(retry.iter()) {
        assert_eq!(before.record_id, after.record_id);
        assert_eq!(before.digest, after.digest);
        assert_eq!(before.canonical, after.canonical);
    }
}
