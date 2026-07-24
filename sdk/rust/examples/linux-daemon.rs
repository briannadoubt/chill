use chill_observability::*;
use std::collections::BTreeMap;
fn main() {
    let mut config = Config::new(
        ServiceName::try_from("inventory.daemon").unwrap(),
        "./chill-queue",
        "https://collector.example/v1/logs",
    );
    config.consent = Consent::Granted;
    let client = Client::new(config).unwrap();
    client.start_session();
    client.event(
        SemanticName::try_from("daemon.ready").unwrap(),
        BTreeMap::new(),
    );
    client.end_session();
}
