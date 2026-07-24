use chill_observability::*;
use std::collections::BTreeMap;
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut config = Config::new(
        ServiceName::try_from("desktop.host").unwrap(),
        "./chill-queue",
        "https://collector.example/v1/logs",
    );
    config.consent = Consent::Granted;
    let client = Client::new(config).unwrap();
    let _: Result<(), ()> = client
        .activity(
            SemanticName::try_from("window.open").unwrap(),
            BTreeMap::new(),
            async { Ok(()) },
        )
        .await;
}
