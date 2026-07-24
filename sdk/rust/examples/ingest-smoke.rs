use chill_observability::{Client, Config, Consent, SemanticName, ServiceName};
use std::{collections::BTreeMap, env, fs, process};
use uuid::Uuid;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let endpoint =
        env::var("CHILL_OTLP_ENDPOINT").unwrap_or_else(|_| fail("CHILL_OTLP_ENDPOINT is required"));
    let credential =
        env::var("CHILL_API_KEY").unwrap_or_else(|_| fail("CHILL_API_KEY is required"));
    let queue = env::temp_dir().join(format!("chill-rust-ingest-{}", Uuid::new_v4()));
    let mut config = Config::new(
        ServiceName::try_from("chill.portable.rust.smoke").unwrap(),
        &queue,
        endpoint,
    );
    config.consent = Consent::Granted;
    config.credential = Some(credential);
    config.policy_version =
        env::var("CHILL_POLICY_VERSION").unwrap_or_else(|_| "privacy-v1".to_owned());
    let client = Client::new(config).unwrap_or_else(|_| fail("portable Rust setup failed"));

    client.start_session();
    client.event(
        SemanticName::try_from("portable.rust.ready").unwrap(),
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
    match client.shutdown().await {
        Ok(exported) if exported >= 5 => {}
        _ => fail("portable Rust export was not acknowledged"),
    }
    let _ = fs::remove_dir_all(&queue);
    println!("portable-rust-ingest-ok");
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    process::exit(1)
}
