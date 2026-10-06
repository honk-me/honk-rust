use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, LOCATION, RETRY_AFTER, USER_AGENT};
use tokio::time::Instant;

use crate::error::{Error, Failure, FieldError, Result};
use crate::message::{
    Accepted, Category, Defaults, EventType, Message, MetadataValue, Priority, Severity,
};
use crate::validate::{encode_message, valid_idempotency_key};

/// The version of this crate, sent in the `User-Agent` header (`honk-me-rust/<version>`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_RETRIES: u32 = 4;
const DEFAULT_DEADLINE: Duration = Duration::from_secs(30);
const DEFAULT_BACKOFF_BASE: Duration = Duration::from_millis(500);
const DEFAULT_BACKOFF_MAX: Duration = Duration::from_secs(8);
const MAX_ANSWER_BYTES: usize = 64 << 10;

/// Sends messages to one Honk project. Cheap to clone (the connection pool is shared); create
/// one per application and reuse it so connections are kept alive. Needs a Tokio runtime; see
/// `blocking::Honk` (feature `blocking`) for synchronous code.
///
/// ```no_run
/// # async fn demo() -> honk_me::Result<()> {
/// use honk_me::Honk;
///
/// let honk = Honk::new("https://honk-me.app", std::env::var("HONK_KEY").unwrap_or_default())?;
/// honk.beep("Backup finished", "nightly pg_dump took 42 s").await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Honk {
    inner: Arc<Inner>,
}

struct Inner {
    endpoint: String,
    key: String,
    timeout: Duration,
    retries: u32,
    deadline: Duration,
    backoff_base: Duration,
    backoff_max: Duration,
    defaults: Defaults,
    validate: bool,
    user_agent: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for Honk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the key.
        f.debug_struct("Honk")
            .field("endpoint", &self.inner.endpoint)
            .finish_non_exhaustive()
    }
}

/// Configures a [`Honk`] client. URL and key are required; everything else has a default.
///
/// ```
/// use std::time::Duration;
/// use honk_me::Honk;
///
/// let honk = Honk::builder()
///     .url("https://honk-me.app")
///     .key("honk_ab12cd34ef56_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345")
///     .timeout(Duration::from_secs(3)) // per attempt (default 5 s)
///     .retries(2)                      // after the first attempt (default 4; 0 disables)
///     .deadline(Duration::from_secs(10)) // total, waits included (default 30 s)
///     .source("billing")              // default source, environment, channel
///     .environment("production")
///     .build()?;
/// # Ok::<(), honk_me::Error>(())
/// ```
#[derive(Debug, Clone)]
#[must_use]
pub struct HonkBuilder {
    url: String,
    key: String,
    timeout: Duration,
    retries: u32,
    deadline: Duration,
    backoff_base: Duration,
    backoff_max: Duration,
    defaults: Defaults,
    validate: bool,
    user_agent: Option<String>,
}

impl Default for HonkBuilder {
    fn default() -> Self {
        HonkBuilder {
            url: String::new(),
            key: String::new(),
            timeout: DEFAULT_TIMEOUT,
            retries: DEFAULT_RETRIES,
            deadline: DEFAULT_DEADLINE,
            backoff_base: DEFAULT_BACKOFF_BASE,
            backoff_max: DEFAULT_BACKOFF_MAX,
            defaults: Defaults::default(),
            validate: true,
            user_agent: None,
        }
    }
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

impl HonkBuilder {
    /// A builder with every default and no URL or key.
    pub fn new() -> Self {
        HonkBuilder::default()
    }

    /// A builder with `HONK_URL`, `HONK_KEY` and the optional `HONK_SOURCE`,
    /// `HONK_ENVIRONMENT` and `HONK_CHANNEL` defaults from the environment.
    pub fn from_env() -> Self {
        HonkBuilder {
            url: env_var("HONK_URL").unwrap_or_default(),
            key: env_var("HONK_KEY").unwrap_or_default(),
            defaults: Defaults {
                source: env_var("HONK_SOURCE"),
                environment: env_var("HONK_ENVIRONMENT"),
                channel: env_var("HONK_CHANNEL"),
            },
            ..HonkBuilder::default()
        }
    }

    /// The base address of the Honk server, e.g. `https://honk-me.app`.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }

    /// A project ingestion key (`honk_…`). Keep it on the server.
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }

    /// The timeout of one HTTP attempt. Default 5 s.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Retries after the first attempt, for network errors, timeouts, 429 and 5xx only.
    /// Default 4; 0 disables retries.
    pub fn retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// The total time budget of one send, waits included. Default 30 s. A wait that would
    /// cross it (a daily quota's `Retry-After`) fails fast instead.
    pub fn deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// The backoff between retries: attempt *n* waits a random time in
    /// `0..=min(max, base·2ⁿ⁻¹)` (full jitter), or the server's `Retry-After` when longer.
    /// Default 500 ms / 8 s.
    pub fn backoff(mut self, base: Duration, max: Duration) -> Self {
        self.backoff_base = base;
        self.backoff_max = max;
        self
    }

    /// Defaults for messages that leave source, environment or channel unset.
    pub fn defaults(mut self, defaults: Defaults) -> Self {
        self.defaults = defaults;
        self
    }

    /// The default source.
    pub fn source(mut self, source: impl Into<String>) -> Self {
        self.defaults.source = Some(source.into());
        self
    }

    /// The default environment.
    pub fn environment(mut self, environment: impl Into<String>) -> Self {
        self.defaults.environment = Some(environment.into());
        self
    }

    /// The default channel.
    pub fn channel(mut self, channel: impl Into<String>) -> Self {
        self.defaults.channel = Some(channel.into());
        self
    }

    /// `false` sends messages without local checks (only "message is required" remains); the
    /// server always validates. Default `true`.
    pub fn validate(mut self, validate: bool) -> Self {
        self.validate = validate;
        self
    }

    /// Appended to the `User-Agent` header, e.g. `my-app/1.2`.
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    /// Checks the configuration and returns the client.
    pub fn build(self) -> Result<Honk> {
        let config = |msg: &str| Error::InvalidConfiguration(msg.to_owned());
        let mut url = self.url.trim().trim_end_matches('/').to_owned();
        if let Some(base) = url.strip_suffix("/v1/messages") {
            url = base.to_owned();
        }
        if url.is_empty() {
            return Err(config(
                "URL is required (the base address of your Honk server, e.g. https://honk-me.app; is HONK_URL set?)",
            ));
        }
        let lower = url.to_ascii_lowercase();
        if !(lower.starts_with("https://") || lower.starts_with("http://"))
            || reqwest::Url::parse(&url).is_err()
        {
            return Err(Error::InvalidConfiguration(format!(
                "URL must be an https:// address (got {:?})",
                self.url
            )));
        }
        let key = self.key.trim().to_owned();
        if key.is_empty() {
            return Err(config(
                "key is required (a project ingestion key honk_…; is HONK_KEY set?)",
            ));
        }
        if !key.starts_with("honk_") || !key.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
            return Err(config(
                "key must be a project ingestion key starting with honk_ (create one under Project → Keys)",
            ));
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .pool_idle_timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| {
                Error::InvalidConfiguration(format!("could not create the HTTP client: {e}"))
            })?;
        let mut user_agent = format!("honk-me-rust/{VERSION}");
        if let Some(extra) = self.user_agent.filter(|s| !s.trim().is_empty()) {
            user_agent.push(' ');
            user_agent.push_str(extra.trim());
        }
        Ok(Honk {
            inner: Arc::new(Inner {
                endpoint: format!("{url}/v1/messages"),
                key,
                timeout: if self.timeout.is_zero() {
                    DEFAULT_TIMEOUT
                } else {
                    self.timeout
                },
                retries: self.retries,
                deadline: if self.deadline.is_zero() {
                    DEFAULT_DEADLINE
                } else {
                    self.deadline
                },
                backoff_base: if self.backoff_base.is_zero() {
                    DEFAULT_BACKOFF_BASE
                } else {
                    self.backoff_base
                },
                backoff_max: if self.backoff_max.is_zero() {
                    DEFAULT_BACKOFF_MAX
                } else {
                    self.backoff_max
                },
                defaults: self.defaults,
                validate: self.validate,
                user_agent,
                http,
            }),
        })
    }

    /// Checks the configuration and returns a synchronous client.
    #[cfg(feature = "blocking")]
    #[cfg_attr(docsrs, doc(cfg(feature = "blocking")))]
    pub fn build_blocking(self) -> Result<crate::blocking::Honk> {
        crate::blocking::Honk::from_async(self.build()?)
    }
}

impl Honk {
    /// A client for `url` (e.g. `https://honk-me.app`) with a project ingestion key and every
    /// default. Fails with [`Error::InvalidConfiguration`] for a missing or malformed URL or key.
    pub fn new(url: impl Into<String>, key: impl Into<String>) -> Result<Honk> {
        HonkBuilder::new().url(url).key(key).build()
    }

    /// A client from `HONK_URL`, `HONK_KEY` and the optional `HONK_SOURCE`,
    /// `HONK_ENVIRONMENT` and `HONK_CHANNEL` defaults.
    pub fn from_env() -> Result<Honk> {
        HonkBuilder::from_env().build()
    }

    /// A [`HonkBuilder`] for the other options.
    pub fn builder() -> HonkBuilder {
        HonkBuilder::new()
    }

    /// Sends one event and returns once Honk has durably stored it (`202`); that does not mean
    /// a push was delivered. Network errors, timeouts, 429 and 5xx are retried with the same
    /// `Idempotency-Key` until the retries or the deadline run out.
    ///
    /// `idempotency_key` is a stable key for this event (1–128 printable ASCII characters,
    /// e.g. `"request-4812"`), or `None` for a new UUIDv7. A replay within 24 hours returns the
    /// original id with `duplicate: true`.
    ///
    /// ```no_run
    /// # async fn demo(honk: honk_me::Honk) -> honk_me::Result<()> {
    /// use honk_me::{Message, Severity};
    ///
    /// let msg = Message::new("Ana asked for a quote: 3 rooms, 2 bathrooms")
    ///     .title("New quote request")
    ///     .severity(Severity::LOUD)
    ///     .group_key("requests/4812")
    ///     .url("https://shop.example.com/admin/requests/4812");
    /// let accepted = honk.send(&msg, "request-4812").await?; // or None for a UUIDv7
    /// println!("{} duplicate={}", accepted.id, accepted.duplicate);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send<'k>(
        &self,
        message: &Message,
        idempotency_key: impl Into<Option<&'k str>>,
    ) -> Result<Accepted> {
        let me = &*self.inner;
        let body = encode_message(message, &me.defaults, me.validate)?;
        let key = match idempotency_key.into() {
            None => crate::uuid::new_idempotency_key(),
            Some(k) if valid_idempotency_key(k) => k.to_owned(),
            Some(_) => {
                return Err(Error::local_validation(vec![FieldError::new(
                    "Idempotency-Key",
                    "invalid_format",
                    "use 1-128 printable ASCII characters without spaces",
                )]));
            }
        };

        let deadline = Instant::now() + me.deadline;
        let mut attempt: u32 = 1;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let timeout = me.timeout.min(remaining).max(Duration::from_millis(1));
            let failure = match self.attempt(&body, &key, timeout, attempt).await {
                Ok(accepted) => return Ok(accepted),
                Err(e) => e,
            };
            if !failure.is_retryable() || attempt > me.retries {
                return Err(failure);
            }
            let mut wait = self.jitter(attempt);
            if let Some(after) = failure.retry_after() {
                wait = wait.max(after);
            }
            if Instant::now() + wait >= deadline {
                return Err(failure);
            }
            tokio::time::sleep(wait).await;
            attempt += 1;
        }
    }

    fn jitter(&self, attempt: u32) -> Duration {
        let me = &*self.inner;
        let exp = me
            .backoff_base
            .saturating_mul(1u32.checked_shl(attempt - 1).unwrap_or(u32::MAX));
        let ceiling = exp.min(me.backoff_max).as_micros() as u64;
        Duration::from_micros(crate::uuid::random_u64() % (ceiling + 1))
    }

    async fn attempt(
        &self,
        body: &str,
        key: &str,
        timeout: Duration,
        attempt: u32,
    ) -> Result<Accepted> {
        let me = &*self.inner;
        let failure = |kind: fn(Box<Failure>) -> Error,
                       code: &str,
                       message: String,
                       source: reqwest::Error| {
            kind(Box::new(Failure {
                code: code.into(),
                message,
                idempotency_key: Some(key.to_owned()),
                attempts: attempt,
                source: Some(Box::new(source)),
                ..Failure::default()
            }))
        };
        let transport = |e: reqwest::Error| {
            if e.is_timeout() {
                failure(
                    Error::Timeout,
                    "timeout",
                    format!(
                        "no answer within {timeout:?} (the event may or may not have been stored; retrying with the same idempotency key is safe)"
                    ),
                    e,
                )
            } else {
                let msg = format!("could not reach Honk: {}", error_chain(&e));
                failure(Error::Network, "network_error", msg, e)
            }
        };

        let mut res = me
            .http
            .post(&me.endpoint)
            .header(AUTHORIZATION, format!("Bearer {}", me.key))
            .header("Idempotency-Key", key)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json")
            .header(USER_AGENT, &me.user_agent)
            .body(body.to_owned())
            .timeout(timeout)
            .send()
            .await
            .map_err(transport)?;
        let status = res.status().as_u16();
        let headers = res.headers().clone();
        let mut data = Vec::new();
        while let Some(chunk) = res.chunk().await.map_err(transport)? {
            data.extend_from_slice(&chunk[..chunk.len().min(MAX_ANSWER_BYTES - data.len())]);
            if data.len() >= MAX_ANSWER_BYTES {
                break;
            }
        }
        if (200..300).contains(&status) {
            return match serde_json::from_slice::<Accepted>(&data) {
                Ok(a) if !a.id.is_empty() => Ok(a),
                _ => Err(Error::Http(Box::new(Failure {
                    status: Some(status),
                    message: "answer without a message id".into(),
                    idempotency_key: Some(key.to_owned()),
                    attempts: attempt,
                    ..Failure::default()
                }))),
            };
        }
        Err(response_error(status, &headers, &data, key, attempt))
    }

    /// Reports a problem for `group_key` (opens or continues its incident). Severity defaults
    /// to long (error); `.severity(…)` overrides it. An empty title lets the server derive one.
    ///
    /// ```no_run
    /// # async fn demo(honk: honk_me::Honk) -> honk_me::Result<()> {
    /// honk.problem("db/backup", "Backup failed", "pg_dump exited with 1").await?;
    /// honk.recovery("db/backup", "Backup OK", "pg_dump finished").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn problem(
        &self,
        group_key: impl Into<String>,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> PendingSend<'_> {
        self.pending(Severity::Error, title, message)
            .group_key(group_key)
            .event_type(EventType::Problem)
    }

    /// Reports that `group_key` recovered (closes its open incident). Severity defaults to beep
    /// (success); `.severity(…)` overrides it.
    pub fn recovery(
        &self,
        group_key: impl Into<String>,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> PendingSend<'_> {
        self.pending(Severity::Success, title, message)
            .group_key(group_key)
            .event_type(EventType::Recovery)
    }

    /// A light honk (severity info). An empty title lets the server derive one from the message.
    ///
    /// Every helper returns a [`PendingSend`]: chain more fields, then `.await` it.
    ///
    /// ```no_run
    /// # async fn demo(honk: honk_me::Honk) -> honk_me::Result<()> {
    /// use honk_me::Priority;
    ///
    /// honk.loud("Disk 91% full", "db-1 /var is at 91%")
    ///     .group_key("disk/db-1/var")
    ///     .priority(Priority::High)
    ///     .idempotency_key("disk-db-1-2026-10-04")
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn light(&self, title: impl Into<String>, message: impl Into<String>) -> PendingSend<'_> {
        self.pending(Severity::Info, title, message)
    }

    /// A beep-beep (severity success).
    pub fn beep(&self, title: impl Into<String>, message: impl Into<String>) -> PendingSend<'_> {
        self.pending(Severity::Success, title, message)
    }

    /// A loud honk (severity warning).
    pub fn loud(&self, title: impl Into<String>, message: impl Into<String>) -> PendingSend<'_> {
        self.pending(Severity::Warning, title, message)
    }

    /// A long honk (severity error; pushes at least as high priority).
    pub fn long(&self, title: impl Into<String>, message: impl Into<String>) -> PendingSend<'_> {
        self.pending(Severity::Error, title, message)
    }

    /// A blast (severity critical; pushes at least as high priority).
    pub fn blast(&self, title: impl Into<String>, message: impl Into<String>) -> PendingSend<'_> {
        self.pending(Severity::Critical, title, message)
    }

    /// A synonym of [`light`](Honk::light).
    pub fn info(&self, title: impl Into<String>, message: impl Into<String>) -> PendingSend<'_> {
        self.light(title, message)
    }

    /// A synonym of [`beep`](Honk::beep).
    pub fn success(&self, title: impl Into<String>, message: impl Into<String>) -> PendingSend<'_> {
        self.beep(title, message)
    }

    /// A synonym of [`loud`](Honk::loud).
    pub fn warning(&self, title: impl Into<String>, message: impl Into<String>) -> PendingSend<'_> {
        self.loud(title, message)
    }

    /// A synonym of [`long`](Honk::long).
    pub fn error(&self, title: impl Into<String>, message: impl Into<String>) -> PendingSend<'_> {
        self.long(title, message)
    }

    /// A synonym of [`blast`](Honk::blast).
    pub fn critical(
        &self,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> PendingSend<'_> {
        self.blast(title, message)
    }

    fn pending(
        &self,
        severity: Severity,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> PendingSend<'_> {
        let title: String = title.into();
        let mut message = Message::new(message).severity(severity);
        if !title.is_empty() {
            message.title = Some(title);
        }
        PendingSend {
            honk: self,
            message,
            idempotency_key: None,
        }
    }
}

/// The source chain of an error in one line ("error sending request: connection refused").
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut next = e.source();
    while let Some(s) = next {
        let text = s.to_string();
        if !out.contains(&text) {
            out.push_str(": ");
            out.push_str(&text);
        }
        next = s.source();
    }
    out
}

/// Reads `Retry-After` as delta-seconds or an HTTP date.
pub(crate) fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    if let Ok(secs) = v.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let at = httpdate::parse_http_date(v).ok()?;
    at.duration_since(now)
        .ok()
        .map(|d| Duration::from_secs(d.as_secs() + u64::from(d.subsec_millis() >= 500)))
}

fn response_error(
    status: u16,
    headers: &reqwest::header::HeaderMap,
    data: &[u8],
    key: &str,
    attempt: u32,
) -> Error {
    #[derive(serde::Deserialize, Default)]
    struct Envelope {
        #[serde(default)]
        error: Body,
    }
    #[derive(serde::Deserialize, Default)]
    struct Body {
        #[serde(default)]
        code: String,
        #[serde(default)]
        message: String,
        #[serde(default)]
        request_id: String,
        #[serde(default)]
        fields: Vec<FieldError>,
    }
    let header = |name| {
        headers
            .get(name)
            .and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok())
    };
    let env: Envelope = serde_json::from_slice(data).unwrap_or_default();
    let mut message = env.error.message;
    if message.is_empty() {
        message = String::from_utf8_lossy(data)
            .trim()
            .chars()
            .take(200)
            .collect();
    }
    if message.is_empty() {
        message = reqwest::StatusCode::from_u16(status)
            .ok()
            .and_then(|s| s.canonical_reason())
            .unwrap_or("unexpected answer")
            .to_owned();
    }
    let request_id = Some(env.error.request_id)
        .filter(|s| !s.is_empty())
        .or_else(|| header("x-request-id").map(str::to_owned));
    let kind: fn(Box<Failure>) -> Error = match status {
        400 | 413 | 415 | 422 => Error::Validation,
        401 | 403 => Error::Auth,
        409 => Error::Conflict,
        429 => Error::Quota,
        500.. => Error::Server,
        300..=399 => {
            message = match header(LOCATION.as_str()) {
                Some(loc) => format!("redirect to {loc}; set the URL to the final https address"),
                None => "redirect; set the URL to the final https address".into(),
            };
            Error::Http
        }
        404 => {
            message.push_str(" (is the URL the base address of your Honk server?)");
            Error::Http
        }
        _ => Error::Http,
    };
    kind(Box::new(Failure {
        status: Some(status),
        code: env.error.code,
        message,
        fields: env.error.fields,
        request_id,
        idempotency_key: Some(key.to_owned()),
        attempts: attempt,
        retry_after: header(RETRY_AFTER.as_str())
            .and_then(|v| parse_retry_after(v, SystemTime::now())),
        ..Failure::default()
    }))
}

/// A send prepared by a helper ([`Honk::loud`], [`Honk::problem`], …). Chain more fields, then
/// `.await` it (or call [`send`](PendingSend::send)).
#[must_use = "a PendingSend does nothing until it is awaited"]
pub struct PendingSend<'a> {
    honk: &'a Honk,
    message: Message,
    idempotency_key: Option<String>,
}

impl std::fmt::Debug for PendingSend<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingSend")
            .field("message", &self.message)
            .field("idempotency_key", &self.idempotency_key)
            .finish()
    }
}

macro_rules! pending_setters {
    ($($(#[$doc:meta])* $name:ident: $t:ty $(=> $conv:ident)?;)+) => {
        $(
            $(#[$doc])*
            pub fn $name(mut self, value: $t) -> Self {
                self.message = self.message.$name(value $(.$conv())?);
                self
            }
        )+
    };
}

impl<'a> PendingSend<'a> {
    /// Uses this stable key instead of a new UUIDv7 (1–128 printable ASCII characters).
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    pending_setters! {
        /// Overrides the severity.
        severity: Severity;
        /// Sets the priority (urgent needs a key with `allow_urgent`).
        priority: Priority;
        /// Sets the category.
        category: Category;
        /// Sets the source (at most 64 characters).
        source: impl Into<String> => into;
        /// Sets the environment (at most 32 characters).
        environment: impl Into<String> => into;
        /// Sets the channel (at most 64 characters).
        channel: impl Into<String> => into;
        /// Sets the group key, e.g. `requests/4812`.
        group_key: impl Into<String> => into;
        /// Sets the event type.
        event_type: EventType;
        /// Sets when the event happened at the source.
        occurred_at: SystemTime;
        /// Sets the https link shown as "Open link".
        url: impl Into<String> => into;
        /// Sets an https image the server fetches after ingestion.
        image_url: impl Into<String> => into;
        /// Sets the push lifetime in seconds (60–86400).
        ttl_seconds: u32;
        /// Sets the monotonic source sequence (needs a group key).
        source_sequence: u64;
    }

    /// Adds a button (at most 3, in display order), e.g. `.action("Call", "tel:+15550134")`.
    pub fn action(mut self, title: impl Into<String>, url: impl Into<String>) -> Self {
        self.message = self.message.action(title, url);
        self
    }

    /// Adds one metadata entry (a string, number or boolean).
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<MetadataValue>) -> Self {
        self.message = self.message.metadata(key, value);
        self
    }

    /// The message as prepared so far.
    pub fn message(&self) -> &Message {
        &self.message
    }

    /// Sends it, like [`Honk::send`].
    pub async fn send(self) -> Result<Accepted> {
        self.honk
            .send(&self.message, self.idempotency_key.as_deref())
            .await
    }

    #[cfg(feature = "blocking")]
    pub(crate) fn into_parts(self) -> (Message, Option<String>) {
        (self.message, self.idempotency_key)
    }
}

impl<'a> IntoFuture for PendingSend<'a> {
    type Output = Result<Accepted>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<Accepted>> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.send())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    #[test]
    fn retry_after_seconds_and_http_dates() {
        let now = UNIX_EPOCH + Duration::from_secs(1_791_116_405);
        assert_eq!(parse_retry_after("7", now), Some(Duration::from_secs(7)));
        assert_eq!(parse_retry_after(" 0 ", now), Some(Duration::ZERO));
        assert_eq!(
            parse_retry_after(&httpdate::fmt_http_date(now + Duration::from_secs(90)), now),
            Some(Duration::from_secs(90))
        );
        assert_eq!(
            parse_retry_after(&httpdate::fmt_http_date(now - Duration::from_secs(90)), now),
            None
        );
        assert_eq!(parse_retry_after("soon", now), None);
        assert_eq!(parse_retry_after("-3", now), None);
        assert_eq!(parse_retry_after("", now), None);
    }

    #[test]
    fn jitter_stays_under_the_ceiling() {
        let honk = Honk::builder()
            .url("https://honk.example.com")
            .key("honk_ab12cd34ef56_ABCDEFGHIJKLMNOPQRSTUVWXYZ012345")
            .backoff(Duration::from_millis(100), Duration::from_millis(250))
            .build()
            .unwrap();
        for attempt in 1..40 {
            let ceiling = Duration::from_millis(100 * (1u64 << (attempt - 1).min(20)))
                .min(Duration::from_millis(250));
            assert!(honk.jitter(attempt) <= ceiling, "attempt {attempt}");
        }
    }
}
