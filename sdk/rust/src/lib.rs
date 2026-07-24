//! Portable, privacy-first semantic observability for Rust processes.
//! It intentionally has no generic `track` API and never inspects callback values.
#![forbid(unsafe_code)]
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashSet},
    fmt, fs, io, panic,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::task_local;
use uuid::Uuid;

const MAGIC: &str = "CHILLQ1";
const SCHEMA: &str = "https://schemas.chill.dev/behavior/v1/envelope.schema.json";
const MAX_SAFE_SEQUENCE: u64 = 9_007_199_254_740_991;
task_local! { static TRACE: RefCell<Option<TraceContext>>; }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServiceName(String);
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SemanticName(String);
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnnotationKey(String);
fn semantic(s: &str) -> Result<String, Error> {
    let mut previous_was_separator = false;
    let valid = !s.is_empty()
        && s.len() <= 128
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes().all(|byte| {
            let separator = matches!(byte, b'.' | b'_' | b'-');
            let accepted = (byte.is_ascii_lowercase() || byte.is_ascii_digit() || separator)
                && !(separator && previous_was_separator);
            previous_was_separator = separator;
            accepted
        })
        && !previous_was_separator;
    if valid {
        Ok(s.into())
    } else {
        Err(Error::InvalidSemanticName)
    }
}
impl TryFrom<&str> for ServiceName {
    type Error = Error;
    fn try_from(s: &str) -> Result<Self, Error> {
        Ok(Self(semantic(s)?))
    }
}
impl TryFrom<&str> for SemanticName {
    type Error = Error;
    fn try_from(s: &str) -> Result<Self, Error> {
        Ok(Self(semantic(s)?))
    }
}
impl TryFrom<&str> for AnnotationKey {
    type Error = Error;
    fn try_from(s: &str) -> Result<Self, Error> {
        let valid = !s.is_empty()
            && s.len() <= 128
            && s.split('.').all(|segment| {
                !segment.is_empty()
                    && segment.as_bytes()[0].is_ascii_lowercase()
                    && segment.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            });
        valid
            .then(|| Self(s.to_owned()))
            .ok_or(Error::InvalidSemanticName)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Classification {
    Public,
    Internal,
    PseudonymousIdentifier,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AnnotationValue {
    String(String),
    Bool(bool),
    Number(f64),
    Strings(Vec<String>),
    Bools(Vec<bool>),
    Numbers(Vec<f64>),
}
impl AnnotationValue {
    fn valid(&self) -> bool {
        match self {
            Self::String(x) => x.len() <= 256,
            Self::Strings(x) => x.len() <= 32 && x.iter().all(|s| s.len() <= 256),
            Self::Bools(x) => x.len() <= 32,
            Self::Numbers(x) => x.len() <= 32 && x.iter().all(|n| n.is_finite()),
            Self::Number(n) => n.is_finite(),
            Self::Bool(_) => true,
        }
    }
    fn json(&self) -> Value {
        match self {
            Self::String(x) => json!(x),
            Self::Bool(x) => json!(x),
            Self::Number(x) => json!(x),
            Self::Strings(x) => json!(x),
            Self::Bools(x) => json!(x),
            Self::Numbers(x) => json!(x),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Annotation {
    pub key: AnnotationKey,
    pub classification: Classification,
}
#[derive(Clone, Debug, Default)]
pub struct AnnotationRegistry {
    keys: BTreeMap<String, Classification>,
}
impl AnnotationRegistry {
    pub fn register(mut self, a: Annotation) -> Self {
        self.keys.insert(a.key.0, a.classification);
        self
    }
    fn validate(
        &self,
        values: &BTreeMap<AnnotationKey, AnnotationValue>,
    ) -> Result<Map<String, Value>, Error> {
        if values.len() > 128 {
            return Err(Error::AnnotationLimit);
        }
        let mut out = Map::new();
        for (key, value) in values {
            if !value.valid() || !self.keys.contains_key(&key.0) {
                return Err(Error::UnregisteredAnnotation);
            }
            out.insert(key.0.clone(), value.json());
        }
        Ok(out)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Consent {
    Granted,
    Denied,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RecordKind {
    Session,
    Action,
    Event,
    Activity,
}
impl RecordKind {
    fn s(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Action => "action",
            Self::Event => "event",
            Self::Activity => "activity",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OutcomeStatus {
    Ok,
    Error,
    Cancelled,
    Timeout,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub status: OutcomeStatus,
    pub reason_code: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraceContext {
    trace_id: String,
    span_id: String,
    flags: String,
}
/// A strict, opaque remote W3C parent accepted by an adapter after its own trust checks.
#[derive(Clone, Debug)]
pub struct RemoteContext(TraceContext);
impl TraceContext {
    pub fn parse(header: &str) -> Option<Self> {
        let p: Vec<_> = header.split('-').collect();
        if p.len() != 4
            || p[0] != "00"
            || p[1].len() != 32
            || p[2].len() != 16
            || p[3].len() != 2
            || !matches!(p[3], "00" | "01")
            || !p
                .iter()
                .skip(1)
                .all(|x| x.bytes().all(|b| b.is_ascii_hexdigit()))
            || p[1].chars().all(|x| x == '0')
            || p[2].chars().all(|x| x == '0')
        {
            None
        } else {
            Some(Self {
                trace_id: p[1].to_ascii_lowercase(),
                span_id: p[2].to_ascii_lowercase(),
                flags: p[3].to_ascii_lowercase(),
            })
        }
    }
    pub fn header(&self) -> String {
        format!("00-{}-{}-{}", self.trace_id, self.span_id, self.flags)
    }
}
impl RemoteContext {
    /// Accepts only a strictly valid version-00 W3C `traceparent` header.
    pub fn from_traceparent(header: &str) -> Result<Self, Error> {
        TraceContext::parse(header)
            .map(Self)
            .ok_or(Error::InvalidTrace)
    }
}
pub enum Error {
    InvalidSemanticName,
    AnnotationLimit,
    UnregisteredAnnotation,
    Io(io::Error),
    Json(serde_json::Error),
    QueueFull,
    InvalidTrace,
    MissingCredential,
    Http(reqwest::Error),
    InvalidEndpoint,
}
impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidSemanticName => "invalid semantic name",
            Self::AnnotationLimit => "annotation limit exceeded",
            Self::UnregisteredAnnotation => "unregistered or invalid annotation",
            Self::Io(_) => "durable queue operation failed",
            Self::Json(_) => "record serialization failed",
            Self::QueueFull => "durable queue is full",
            Self::InvalidTrace => "invalid trace context",
            Self::MissingCredential => "export credential is missing",
            Self::Http(_) => "OTLP export failed",
            Self::InvalidEndpoint => "OTLP endpoint is invalid",
        };
        formatter.write_str(message)
    }
}
impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

#[derive(Clone)]
pub struct Config {
    pub service: ServiceName,
    pub queue_dir: PathBuf,
    pub endpoint: String,
    pub installation_id: Uuid,
    pub process_id: Uuid,
    pub annotations: AnnotationRegistry,
    pub consent: Consent,
    pub max_queue_bytes: u64,
    pub trusted_origins: HashSet<String>,
    pub credential: Option<String>,
    pub policy_version: String,
}
impl Config {
    pub fn new(
        service: ServiceName,
        queue_dir: impl Into<PathBuf>,
        endpoint: impl Into<String>,
    ) -> Self {
        Self {
            service,
            queue_dir: queue_dir.into(),
            endpoint: endpoint.into(),
            installation_id: Uuid::new_v4(),
            process_id: Uuid::new_v4(),
            annotations: AnnotationRegistry::default(),
            consent: Consent::Denied,
            max_queue_bytes: 64 * 1024 * 1024,
            trusted_origins: HashSet::new(),
            credential: None,
            policy_version: "privacy-v1".into(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Stored {
    magic: String,
    record: Value,
}
#[derive(Clone)]
pub struct Queue {
    dir: PathBuf,
    limit: u64,
}
impl Queue {
    pub fn open(dir: impl Into<PathBuf>, limit: u64) -> Result<Self, Error> {
        let q = Self {
            dir: dir.into(),
            limit,
        };
        fs::create_dir_all(&q.dir)?;
        q.recover()?;
        Ok(q)
    }
    fn files(&self) -> Result<Vec<PathBuf>, Error> {
        let mut x = vec![];
        for e in fs::read_dir(&self.dir)? {
            let p = e?.path();
            if p.extension().is_some_and(|v| v == "json") {
                x.push(p)
            }
        }
        x.sort();
        Ok(x)
    }
    fn recover(&self) -> Result<(), Error> {
        for p in self.files()? {
            let ok = fs::read(&p)
                .ok()
                .and_then(|b| serde_json::from_slice::<Stored>(&b).ok())
                .is_some_and(|s| s.magic == MAGIC);
            if !ok {
                let _ = fs::rename(&p, p.with_extension("corrupt"));
            }
        }
        Ok(())
    }
    pub fn push(&self, record: &Value) -> Result<(), Error> {
        let bytes = serde_json::to_vec(&Stored {
            magic: MAGIC.into(),
            record: record.clone(),
        })?;
        if self.bytes()? + bytes.len() as u64 > self.limit {
            return Err(Error::QueueFull);
        }
        let id = record["record_id"].as_str().ok_or(Error::QueueFull)?;
        let tmp = self.dir.join(format!(".{id}.tmp"));
        let out = self.dir.join(format!("{id}.json"));
        use std::io::Write;
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(tmp, out)?;
        fs::File::open(&self.dir)?.sync_all()?;
        Ok(())
    }
    pub fn peek(&self, n: usize) -> Result<Vec<(PathBuf, Value)>, Error> {
        self.files()?
            .into_iter()
            .take(n)
            .filter_map(|p| {
                fs::read(&p)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Stored>(&b).ok())
                    .map(|s| (p, s.record))
            })
            .collect::<Vec<_>>()
            .pipe(Ok)
    }
    pub fn ack(&self, paths: &[PathBuf]) -> Result<(), Error> {
        for p in paths {
            if p.parent() == Some(self.dir.as_path()) {
                match fs::remove_file(p) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(Error::Io(error)),
                }
            }
        }
        Ok(())
    }
    pub fn bytes(&self) -> Result<u64, Error> {
        Ok(self
            .files()?
            .iter()
            .filter_map(|p| fs::metadata(p).ok().map(|m| m.len()))
            .sum())
    }
}
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}
struct Inner {
    config: Config,
    queue: Queue,
    consent: Mutex<Consent>,
    session: Mutex<Option<Uuid>>,
    sequence: AtomicU64,
    panic_reporting: AtomicBool,
    panic_hook_installed: AtomicBool,
}
impl Client {
    pub fn new(config: Config) -> Result<Self, Error> {
        validate_endpoint(&config.endpoint)?;
        let q = Queue::open(&config.queue_dir, config.max_queue_bytes)?;
        Ok(Self {
            inner: Arc::new(Inner {
                consent: Mutex::new(config.consent),
                config,
                queue: q,
                session: Mutex::new(None),
                sequence: AtomicU64::new(0),
                panic_reporting: AtomicBool::new(false),
                panic_hook_installed: AtomicBool::new(false),
            }),
        })
    }
    pub fn set_consent(&self, c: Consent) -> Result<(), Error> {
        *self.inner.consent.lock().unwrap() = c;
        if c == Consent::Denied {
            *self.inner.session.lock().unwrap() = None;
            for path in self.inner.queue.files()? {
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(Error::Io(error)),
                }
            }
        }
        Ok(())
    }
    pub fn start_session(&self) -> Option<Uuid> {
        if *self.inner.consent.lock().unwrap() != Consent::Granted {
            return None;
        }
        let mut session = self.inner.session.lock().unwrap();
        if let Some(id) = *session {
            return Some(id);
        }
        let id = Uuid::now_v7();
        *session = Some(id);
        drop(session);
        self.emit_with(
            RecordKind::Session,
            "start",
            SemanticName("process.session".into()),
            BTreeMap::new(),
            None,
            Some(id),
            None,
            None,
        );
        Some(id)
    }
    pub fn end_session(&self) {
        let session = self.inner.session.lock().unwrap().take();
        let Some(session) = session else { return };
        self.emit_with(
            RecordKind::Session,
            "end",
            SemanticName("process.session".into()),
            BTreeMap::new(),
            Some(Outcome {
                status: OutcomeStatus::Ok,
                reason_code: None,
            }),
            Some(session),
            None,
            None,
        );
    }
    pub fn event(
        &self,
        name: SemanticName,
        annotations: BTreeMap<AnnotationKey, AnnotationValue>,
    ) -> Option<Uuid> {
        self.emit_with(
            RecordKind::Event,
            "instant",
            name,
            annotations,
            None,
            None,
            None,
            None,
        )
    }
    pub fn action(
        &self,
        name: SemanticName,
        annotations: BTreeMap<AnnotationKey, AnnotationValue>,
    ) -> Option<Uuid> {
        self.emit_with(
            RecordKind::Action,
            "instant",
            name,
            annotations,
            None,
            None,
            None,
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn emit_with(
        &self,
        kind: RecordKind,
        op: &str,
        name: SemanticName,
        annotations: BTreeMap<AnnotationKey, AnnotationValue>,
        outcome: Option<Outcome>,
        subject: Option<Uuid>,
        trace: Option<TraceContext>,
        duration_nano: Option<u128>,
    ) -> Option<Uuid> {
        if *self.inner.consent.lock().unwrap() != Consent::Granted {
            return None;
        }
        let attrs = self.inner.config.annotations.validate(&annotations).ok()?;
        let id = Uuid::now_v7();
        let session = *self.inner.session.lock().unwrap();
        if !matches!(kind, RecordKind::Session) && session.is_none() {
            return None;
        }
        let mut m = Map::new();
        m.insert("record_id".into(), json!(id));
        m.insert(
            "subject_id".into(),
            json!(subject.unwrap_or_else(|| {
                if matches!(kind, RecordKind::Action | RecordKind::Event) {
                    id
                } else {
                    Uuid::now_v7()
                }
            })),
        );
        m.insert("kind".into(), json!(kind.s()));
        m.insert("operation".into(), json!(op));
        m.insert("name".into(), json!(name.0));
        m.insert(
            "occurred_at_unix_nano".into(),
            json!(now_nano().to_string()),
        );
        m.insert("annotations".into(), Value::Object(attrs));
        let sequence = self
            .inner
            .sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                (current < MAX_SAFE_SEQUENCE).then_some(current + 1)
            })
            .ok()?
            + 1;
        m.insert("sequence_number".into(), json!(sequence));
        m.insert(
            "observed_at_unix_nano".into(),
            json!(now_nano().to_string()),
        );
        m.insert(
            "installation_id".into(),
            json!(self.inner.config.installation_id),
        );
        m.insert("process_id".into(), json!(self.inner.config.process_id));
        let context_session =
            session.or_else(|| matches!(kind, RecordKind::Session).then_some(subject?));
        if let Some(session) = context_session {
            m.insert("session_id".into(), json!(session));
        }
        if let Some(duration) = duration_nano {
            m.insert("duration_nano".into(), json!(duration.to_string()));
        }
        if let Some(o) = outcome {
            m.insert("outcome".into(), serde_json::to_value(o).ok()?);
        }
        if let Some(t) = trace.or_else(|| TRACE.try_with(|x| x.borrow().clone()).ok().flatten()) {
            m.insert("traceparent".into(), json!(t.header()));
        }
        let r = Value::Object(m);
        self.inner.queue.push(&r).ok()?;
        Some(id)
    }
    pub async fn activity<T, E, F>(
        &self,
        name: SemanticName,
        annotations: BTreeMap<AnnotationKey, AnnotationValue>,
        work: F,
    ) -> Result<T, E>
    where
        F: std::future::Future<Output = Result<T, E>>,
    {
        if *self.inner.consent.lock().unwrap() != Consent::Granted {
            return work.await;
        }
        let parent = TRACE.try_with(|x| x.borrow().clone()).ok().flatten();
        let trace = parent
            .map(|p| TraceContext {
                trace_id: p.trace_id,
                span_id: hex_id(8),
                flags: p.flags,
            })
            .unwrap_or_else(|| TraceContext {
                trace_id: hex_id(16),
                span_id: hex_id(8),
                flags: "01".into(),
            });
        let subject = Uuid::now_v7();
        let started = Instant::now();
        self.emit_with(
            RecordKind::Activity,
            "start",
            name.clone(),
            annotations.clone(),
            None,
            Some(subject),
            Some(trace.clone()),
            None,
        );
        let result = TRACE.scope(RefCell::new(Some(trace.clone())), work).await;
        let status = if result.is_ok() {
            OutcomeStatus::Ok
        } else {
            OutcomeStatus::Error
        };
        self.emit_with(
            RecordKind::Activity,
            "end",
            name,
            annotations,
            Some(Outcome {
                status,
                reason_code: None,
            }),
            Some(subject),
            Some(trace),
            Some(started.elapsed().as_nanos()),
        );
        result
    }
    /// Runs synchronous application work with an adapter-validated remote parent.
    /// The context is scoped to this call and neither application arguments nor results are observed.
    pub fn with_remote_context<T>(&self, context: &RemoteContext, work: impl FnOnce() -> T) -> T {
        TRACE.sync_scope(RefCell::new(Some(context.0.clone())), work)
    }
    /// Runs an activity joined to an adapter-validated remote parent.
    pub async fn activity_with_context<T, E, F>(
        &self,
        context: &RemoteContext,
        name: SemanticName,
        annotations: BTreeMap<AnnotationKey, AnnotationValue>,
        work: F,
    ) -> Result<T, E>
    where
        F: std::future::Future<Output = Result<T, E>>,
    {
        TRACE
            .scope(
                RefCell::new(Some(context.0.clone())),
                self.activity(name, annotations, work),
            )
            .await
    }
    pub fn inject_traceparent(&self, origin: &str, headers: &mut BTreeMap<String, String>) {
        if !self.inner.config.trusted_origins.contains(origin) {
            return;
        }
        if let Ok(Some(t)) = TRACE.try_with(|x| x.borrow().clone()) {
            headers.insert("traceparent".into(), t.header());
        }
    }
    pub fn extract_trusted(&self, origin: &str, header: &str) -> Option<TraceContext> {
        self.inner
            .config
            .trusted_origins
            .contains(origin)
            .then(|| TraceContext::parse(header))
            .flatten()
    }
    pub fn queue(&self) -> Queue {
        self.inner.queue.clone()
    }
    /// Sends one bounded batch and removes entries only after a successful 2xx acknowledgement.
    pub async fn flush(&self) -> Result<usize, Error> {
        let credential = self
            .inner
            .config
            .credential
            .as_deref()
            .ok_or(Error::MissingCredential)?;
        let batch = self.inner.queue.peek(100)?;
        if batch.is_empty() {
            return Ok(0);
        }
        let records: Vec<Value> = batch.iter().map(|(_, record)| record.clone()).collect();
        let response = reqwest::Client::new()
            .post(&self.inner.config.endpoint)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {credential}"),
            )
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("X-Chill-Schema-Version", "1.0.0")
            .json(&self.otlp_json(&records))
            .send()
            .await?;
        if response.status().is_success() {
            self.inner
                .queue
                .ack(&batch.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>())?;
            Ok(batch.len())
        } else {
            Ok(0)
        }
    }
    pub async fn shutdown(&self) -> Result<usize, Error> {
        self.flush().await
    }
    pub fn install_panic_hook(&self) {
        if self.inner.panic_hook_installed.swap(true, Ordering::AcqRel) {
            return;
        }
        let prior = panic::take_hook();
        let c = self.clone();
        panic::set_hook(Box::new(move |info| {
            if !c.inner.panic_reporting.swap(true, Ordering::AcqRel) {
                c.event(SemanticName("process.panic".into()), BTreeMap::new());
            }
            prior(info)
        }));
    }
    pub fn otlp_json(&self, records: &[Value]) -> Value {
        let attrs = records
            .iter()
            .map(|r| {
                let mut a = vec![
                    kv("chill.schema.version", json!("1.0.0")),
                    kv("chill.schema.url", json!(SCHEMA)),
                    kv("chill.record.id", r["record_id"].clone()),
                    kv("chill.subject.id", r["subject_id"].clone()),
                    kv("chill.behavior.kind", r["kind"].clone()),
                    kv("chill.behavior.operation", r["operation"].clone()),
                    kv("chill.behavior.name", r["name"].clone()),
                    kv_int("chill.clock.sequence_number", &r["sequence_number"]),
                    kv("chill.source.platform", json!("server")),
                    kv("chill.source.installation_id", r["installation_id"].clone()),
                    kv("chill.source.process_id", r["process_id"].clone()),
                    kv("chill.privacy.consent", json!("granted")),
                    kv("chill.privacy.policy_version", json!(self.inner.config.policy_version)),
                    kv("chill.privacy.capture_class", json!("analytics")),
                    kv("chill.privacy.redaction_state", json!("none")),
                ];
                if let Some(session) = r.get("session_id") { a.push(kv("chill.context.session_id",session.clone())); }
                match r["kind"].as_str() {
                    Some("activity") => { a.push(kv("chill.payload.activity_kind", json!("custom"))); a.push(kv("chill.payload.role", json!("operation"))); a.push(kv_int("chill.payload.attempt", &json!(1))); a.push(kv_int("chill.payload.recursion_depth", &json!(0))); }
                    Some("event") => { a.push(kv("chill.payload.event_class", json!("custom"))); a.push(kv("chill.payload.emission", json!("observed"))); }
                    Some("action") => { a.push(kv("chill.payload.element_id", r["name"].clone())); a.push(kv("chill.payload.role", json!("operation"))); a.push(kv("chill.payload.activation", json!("system"))); a.push(kv("chill.payload.input", json!("system"))); }
                    _ => {}
                }
                if let Some(outcome) = r.get("outcome") && let Some(status) = outcome.get("status").and_then(Value::as_str) { a.push(kv("chill.outcome.status", json!(status.to_lowercase()))); }
                if let Some(duration) = r.get("duration_nano") { a.push(kv("chill.duration_nano", duration.clone())); }
                if let Some(annotations)=r.get("annotations").and_then(Value::as_object) { for (key,value) in annotations { a.push(kv(&format!("chill.annotation.{key}"),value.clone())); if let Some(class)=self.inner.config.annotations.keys.get(key) { a.push(kv(&format!("chill.privacy.annotation_classification.{key}"),json!(classification_name(class)))); } } }
                let mut log=json!({"timeUnixNano":r["occurred_at_unix_nano"],"observedTimeUnixNano":r["observed_at_unix_nano"],"eventName":r["name"],"attributes":a});
                if let Some(t)=r.get("traceparent").and_then(Value::as_str).and_then(TraceContext::parse) { log["traceId"]=json!(t.trace_id); log["spanId"]=json!(t.span_id); log["flags"]=json!(u8::from_str_radix(&t.flags,16).unwrap_or(0)); }
                log
            })
            .collect::<Vec<_>>();
        json!({"resourceLogs":[{"resource":{"attributes":[kv("service.name",json!(self.inner.config.service.0)),kv("process.runtime.name",json!("rust")),kv("process.runtime.version",json!(env!("CARGO_PKG_VERSION"))),kv("os.type",json!(std::env::consts::OS))]},"scopeLogs":[{"scope":{"name":"dev.chill.rust","version":env!("CARGO_PKG_VERSION")},"schemaUrl":SCHEMA,"logRecords":attrs}]}]})
    }
}
fn kv(k: &str, v: Value) -> Value {
    json!({"key":k,"value":any_value(v)})
}
fn kv_int(k: &str, v: &Value) -> Value {
    let decimal = match v {
        Value::String(x) => x.clone(),
        Value::Number(x) => x.to_string(),
        _ => "0".into(),
    };
    json!({"key":k,"value":{"intValue":decimal}})
}
fn any_value(v: Value) -> Value {
    match v {
        Value::String(x) => json!({"stringValue":x}),
        Value::Bool(x) => json!({"boolValue":x}),
        Value::Number(x) => {
            if x.is_i64() || x.is_u64() {
                json!({"intValue":x.to_string()})
            } else {
                json!({"doubleValue":x})
            }
        }
        Value::Array(x) => {
            json!({"arrayValue":{"values":x.into_iter().map(any_value).collect::<Vec<_>>()}})
        }
        _ => json!({"stringValue":""}),
    }
}
fn classification_name(c: &Classification) -> &'static str {
    match c {
        Classification::Public => "public",
        Classification::Internal => "internal",
        Classification::PseudonymousIdentifier => "pseudonymous_identifier",
    }
}
fn now_nano() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos()
}
fn hex_id(bytes: usize) -> String {
    loop {
        let mut raw = vec![0u8; bytes];
        getrandom::fill(&mut raw).expect("OS randomness unavailable");
        if raw.iter().any(|byte| *byte != 0) {
            return raw.iter().map(|byte| format!("{byte:02x}")).collect();
        }
    }
}
fn validate_endpoint(endpoint: &str) -> Result<(), Error> {
    let url = reqwest::Url::parse(endpoint).map_err(|_| Error::InvalidEndpoint)?;
    let loopback = matches!(
        url.host_str(),
        Some("localhost") | Some("127.0.0.1") | Some("[::1]") | Some("::1")
    );
    let transport_allowed = url.scheme() == "https" || (url.scheme() == "http" && loopback);
    if transport_allowed
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/v1/logs"
        && url.query().is_none()
        && url.fragment().is_none()
    {
        Ok(())
    } else {
        Err(Error::InvalidEndpoint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    fn client() -> Client {
        let d = tempdir().unwrap().keep();
        let mut c = Config::new(
            ServiceName::try_from("daemon").unwrap(),
            d,
            "http://localhost:4318/v1/logs",
        );
        c.consent = Consent::Granted;
        c.annotations = AnnotationRegistry::default().register(Annotation {
            key: AnnotationKey::try_from("mode").unwrap(),
            classification: Classification::Public,
        });
        Client::new(c).unwrap()
    }
    #[test]
    fn privacy_and_consent_purge() {
        let c = client();
        c.start_session();
        assert!(
            c.event(
                SemanticName::try_from("safe.event").unwrap(),
                BTreeMap::new()
            )
            .is_some()
        );
        c.set_consent(Consent::Denied).unwrap();
        assert_eq!(c.queue().peek(10).unwrap().len(), 0);
        assert!(
            c.event(
                SemanticName::try_from("safe.event").unwrap(),
                BTreeMap::new()
            )
            .is_none()
        );
    }
    #[test]
    fn queue_recovery_ack_limit_corrupt() {
        let d = tempdir().unwrap();
        let q = Queue::open(d.path(), 10000).unwrap();
        let r = json!({"record_id":Uuid::now_v7()});
        q.push(&r).unwrap();
        let p = q.peek(1).unwrap()[0].0.clone();
        q.ack(&[p]).unwrap();
        assert!(q.peek(1).unwrap().is_empty());
        fs::write(d.path().join("bad.json"), b"secret-canary").unwrap();
        Queue::open(d.path(), 10000).unwrap();
        assert!(d.path().join("bad.corrupt").exists());
        assert!(matches!(
            Queue::open(d.path(), 1).unwrap().push(&r),
            Err(Error::QueueFull)
        ));
    }
    #[test]
    fn trace_trust() {
        assert!(
            TraceContext::parse("00-00000000000000000000000000000000-1234567890abcdef-01")
                .is_none()
        );
        let c = client();
        let mut h = BTreeMap::new();
        c.inject_traceparent("https://no", &mut h);
        assert!(h.is_empty());
        assert_eq!(hex_id(16).len(), 32);
        assert_ne!(hex_id(8), "0000000000000000");
    }
    #[test]
    fn names_endpoints_and_sessions_are_strict() {
        assert!(SemanticName::try_from("1.bad").is_err());
        assert!(SemanticName::try_from(".bad").is_err());
        assert!(SemanticName::try_from("bad..name").is_err());
        assert!(SemanticName::try_from("bad-").is_err());
        assert!(AnnotationKey::try_from("bad-key").is_err());
        let d = tempdir().unwrap();
        let config = Config::new(
            ServiceName::try_from("daemon").unwrap(),
            d.path(),
            "http://example.test/v1/logs",
        );
        assert!(matches!(Client::new(config), Err(Error::InvalidEndpoint)));
        let config = Config::new(
            ServiceName::try_from("daemon").unwrap(),
            d.path(),
            "https://example.test/not-logs",
        );
        assert!(matches!(Client::new(config), Err(Error::InvalidEndpoint)));
        let c = client();
        let one = c.start_session().unwrap();
        let two = c.start_session().unwrap();
        assert_eq!(one, two);
        c.end_session();
        c.end_session();
    }
    #[tokio::test]
    async fn activity_outcome() {
        let c = client();
        c.start_session();
        let _: Result<(), ()> = c
            .activity(
                SemanticName::try_from("work.run").unwrap(),
                BTreeMap::new(),
                async { Ok(()) },
            )
            .await;
        assert_eq!(c.queue().peek(10).unwrap().len(), 3);
    }
    #[test]
    fn lifecycle_panic_hook_chains() {
        let c = client();
        c.start_session();
        c.end_session();
        c.install_panic_hook();
        assert!(c.queue().peek(10).unwrap().len() >= 2);
    }
    #[test]
    fn otlp_projection_has_typed_required_contract() {
        let c = client();
        c.start_session();
        c.event(
            SemanticName::try_from("daemon.ready").unwrap(),
            BTreeMap::new(),
        );
        c.action(
            SemanticName::try_from("daemon.submit").unwrap(),
            BTreeMap::new(),
        );
        let records: Vec<_> = c
            .queue()
            .peek(10)
            .unwrap()
            .into_iter()
            .map(|(_, r)| r)
            .collect();
        let request = c.otlp_json(&records);
        let logs = &request["resourceLogs"][0]["scopeLogs"][0]["logRecords"];
        assert_eq!(logs.as_array().unwrap().len(), 3);
        let attrs = logs[0]["attributes"].as_array().unwrap();
        assert!(
            attrs
                .iter()
                .any(|v| v["key"] == "chill.source.platform"
                    && v["value"]["stringValue"] == "server")
        );
        assert!(attrs.iter().any(
            |v| v["key"] == "chill.clock.sequence_number" && v["value"]["intValue"].is_string()
        ));
        assert!(
            attrs
                .iter()
                .any(|v| v["key"] == "chill.privacy.capture_class"
                    && v["value"]["stringValue"] == "analytics")
        );
        assert!(
            attrs
                .iter()
                .any(|v| v["key"] == "chill.privacy.redaction_state"
                    && v["value"]["stringValue"] == "none")
        );
        let event = logs[1]["attributes"].as_array().unwrap();
        assert!(
            event.iter().any(|v| v["key"] == "chill.payload.event_class"
                && v["value"]["stringValue"] == "custom")
        );
        assert!(event.iter().any(
            |v| v["key"] == "chill.payload.emission" && v["value"]["stringValue"] == "observed"
        ));
        let action = logs[2]["attributes"].as_array().unwrap();
        assert!(action.iter().any(|v| v["key"] == "chill.payload.element_id"
            && v["value"]["stringValue"] == "daemon.submit"));
        assert!(
            action
                .iter()
                .any(|v| v["key"] == "chill.payload.role"
                    && v["value"]["stringValue"] == "operation")
        );
        assert_eq!(
            request["resourceLogs"][0]["resource"]["attributes"][0]["key"],
            "service.name"
        );
        assert_eq!(
            request["resourceLogs"][0]["scopeLogs"][0]["schemaUrl"],
            SCHEMA
        );
    }
}
