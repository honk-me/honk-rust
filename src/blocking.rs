//! A synchronous client (feature `blocking`), for scripts, CLIs and code without an async
//! runtime. It wraps the async [`Honk`](crate::Honk) on a private single-threaded Tokio runtime,
//! so it has the same retries, deadline, idempotency and validation.
//!
//! Don't call it from inside an async runtime (Tokio panics on nested runtimes); use the async
//! client there.
//!
//! ```no_run
//! use honk_me::blocking::Honk;
//!
//! let honk = Honk::from_env()?; // HONK_URL, HONK_KEY
//! honk.beep("Backup finished", "nightly pg_dump took 42 s").send()?;
//! honk.problem("db/backup", "Backup failed", "pg_dump exited with 1")
//!     .idempotency_key("backup-2026-10-04")
//!     .send()?;
//! # Ok::<(), honk_me::Error>(())
//! ```

use std::sync::Arc;
use std::time::SystemTime;

use crate::error::{Error, Result};
use crate::message::{Accepted, Category, EventType, Message, MetadataValue, Priority, Severity};

/// The synchronous counterpart of [`crate::Honk`]. Cheap to clone; safe to share between
/// threads.
#[derive(Clone, Debug)]
pub struct Honk {
    inner: crate::Honk,
    runtime: Arc<tokio::runtime::Runtime>,
}

impl Honk {
    /// A client for `url` with a project ingestion key and every default.
    pub fn new(url: impl Into<String>, key: impl Into<String>) -> Result<Honk> {
        crate::HonkBuilder::new().url(url).key(key).build_blocking()
    }

    /// A client from `HONK_URL`, `HONK_KEY` and the optional `HONK_SOURCE`,
    /// `HONK_ENVIRONMENT` and `HONK_CHANNEL` defaults.
    pub fn from_env() -> Result<Honk> {
        crate::HonkBuilder::from_env().build_blocking()
    }

    /// A [`HonkBuilder`](crate::HonkBuilder); finish it with
    /// [`build_blocking`](crate::HonkBuilder::build_blocking).
    pub fn builder() -> crate::HonkBuilder {
        crate::HonkBuilder::new()
    }

    pub(crate) fn from_async(inner: crate::Honk) -> Result<Honk> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| {
                Error::InvalidConfiguration(format!("could not start the blocking runtime: {e}"))
            })?;
        Ok(Honk {
            inner,
            runtime: Arc::new(runtime),
        })
    }

    /// Sends one event and blocks until Honk has stored it; see [`crate::Honk::send`].
    pub fn send<'k>(
        &self,
        message: &Message,
        idempotency_key: impl Into<Option<&'k str>>,
    ) -> Result<Accepted> {
        self.runtime
            .block_on(self.inner.send(message, idempotency_key))
    }

    fn pending(&self, p: crate::PendingSend<'_>) -> PendingSend<'_> {
        let (message, idempotency_key) = p.into_parts();
        PendingSend {
            honk: self,
            message,
            idempotency_key,
        }
    }

    /// Reports a problem for `group_key`; see [`crate::Honk::problem`].
    pub fn problem(
        &self,
        group_key: impl Into<String>,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> PendingSend<'_> {
        self.pending(self.inner.problem(group_key, title, message))
    }

    /// Reports that `group_key` recovered; see [`crate::Honk::recovery`].
    pub fn recovery(
        &self,
        group_key: impl Into<String>,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> PendingSend<'_> {
        self.pending(self.inner.recovery(group_key, title, message))
    }

    /// A light honk (severity info).
    pub fn light(&self, title: impl Into<String>, message: impl Into<String>) -> PendingSend<'_> {
        self.pending(self.inner.light(title, message))
    }

    /// A beep-beep (severity success).
    pub fn beep(&self, title: impl Into<String>, message: impl Into<String>) -> PendingSend<'_> {
        self.pending(self.inner.beep(title, message))
    }

    /// A loud honk (severity warning).
    pub fn loud(&self, title: impl Into<String>, message: impl Into<String>) -> PendingSend<'_> {
        self.pending(self.inner.loud(title, message))
    }

    /// A long honk (severity error).
    pub fn long(&self, title: impl Into<String>, message: impl Into<String>) -> PendingSend<'_> {
        self.pending(self.inner.long(title, message))
    }

    /// A blast (severity critical).
    pub fn blast(&self, title: impl Into<String>, message: impl Into<String>) -> PendingSend<'_> {
        self.pending(self.inner.blast(title, message))
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
}

/// A send prepared by a helper of the blocking client. Chain more fields, then call
/// [`send`](PendingSend::send).
#[derive(Debug)]
#[must_use = "a PendingSend does nothing until send() is called"]
pub struct PendingSend<'a> {
    honk: &'a Honk,
    message: Message,
    idempotency_key: Option<String>,
}

macro_rules! blocking_setters {
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

impl PendingSend<'_> {
    /// Uses this stable key instead of a new UUIDv7 (1–128 printable ASCII characters).
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    blocking_setters! {
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

    /// Adds one metadata entry (a string, number or boolean).
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<MetadataValue>) -> Self {
        self.message = self.message.metadata(key, value);
        self
    }

    /// The message as prepared so far.
    pub fn message(&self) -> &Message {
        &self.message
    }

    /// Sends it and blocks until Honk has stored it.
    pub fn send(self) -> Result<Accepted> {
        self.honk
            .send(&self.message, self.idempotency_key.as_deref())
    }
}
