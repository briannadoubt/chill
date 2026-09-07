//! OTLP/HTTP JSON emitted by the source-only Unreal Engine plugin.
//!
//! The fixture is captured from the shipping plugin: the automation test
//! `Chill.Unreal.Fixture.DumpEnvelope` builds the batch through the real client and
//! writes the envelope, and only the volatile identifiers (record, trace, span,
//! session, boot and process ids) are pinned so the fixture is stable.
#![allow(clippy::unwrap_used, reason = "test setup and assertions fail fast")]

use chill_normalize::{Decoder, InboxItem, TimingPolicy};
use serde_json::json;
use time::{Duration, OffsetDateTime};

const UNREAL_OTLP: &[u8] = include_bytes!("../../../testdata/unreal-sdk-otlp.json");
const BEHAVIOR_SCHEMA: &str = include_str!("../../../../schemas/behavior/v1/envelope.schema.json");

fn item() -> InboxItem {
    InboxItem {
        id: 107,
        organization_id: "18e9a1b3-2a84-4b4e-90f5-d5039ae2eb60".to_owned(),
        project_id: "81be8b89-68bc-4fd6-a8d1-a956b8e568bb".to_owned(),
        environment_id: "8a0c2ddf-e5a5-4f55-bcaa-c5aa6fc2733c".to_owned(),
        data_source_id: "a7377206-3c43-464e-8dbb-682dcabde42d".to_owned(),
        signal_kind: "logs".to_owned(),
        payload_format: "json".to_owned(),
        payload: UNREAL_OTLP.to_vec(),
        metadata: json!({"sdk":"unreal"}),
        server_received_at: OffsetDateTime::UNIX_EPOCH + Duration::seconds(1_800_000_000),
        attempt_count: 1,
    }
}

#[test]
fn unreal_generated_otlp_passes_the_production_normalizer() {
    let records = Decoder::new(TimingPolicy::default())
        .decode(&item())
        .unwrap();
    let schema: serde_json::Value = serde_json::from_str(BEHAVIOR_SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for record in &records {
        validator.validate(&record.canonical).unwrap();
    }

    assert_eq!(records.len(), 2);
    assert_eq!(records[0].canonical["kind"], "session");
    assert_eq!(records[0].canonical["operation"], "start");
    assert_eq!(records[1].canonical["kind"], "event");
    assert_eq!(records[1].canonical["name"], "match.started");
    assert_eq!(records[1].canonical["payload"]["event_class"], "domain");
    assert_eq!(records[1].canonical["payload"]["severity"], "info");
    assert_eq!(records[1].canonical["payload"]["emission"], "observed");

    for record in &records {
        assert_eq!(record.canonical["source"]["platform"], "server");
        assert_eq!(
            record.canonical["resource"]["attributes"]["telemetry.sdk.language"],
            "cpp"
        );
        assert_eq!(
            record.canonical["resource"]["attributes"]["process.runtime.name"],
            "unreal"
        );
        assert_eq!(
            record.canonical["resource"]["attributes"]["chill.game.engine"],
            "unreal"
        );
        assert_eq!(
            record.canonical["clock"]["boot_id"],
            "5c2b9a17-4d3e-4a92-b8f1-6e07c4d2a930"
        );
        assert_eq!(record.canonical["clock"]["monotonic_nano"], "1000000");
    }

    // The declared annotation survives; the SDK key never reaches the wire.
    assert_eq!(records[1].canonical["annotations"]["game.mode"], "ranked");

    let canonical = records
        .iter()
        .map(|record| serde_json::to_string(&record.canonical).unwrap())
        .collect::<String>();
    assert!(!canonical.contains("CHILL_API_KEY"));
    assert!(!canonical.contains("canary"));
}
