//! The official Rust client for [Honk](https://honk-me.app): send events from your apps, jobs,
//! scripts and CI to your phone, as calm, grouped push notifications.
//!
//! - Async-first on Tokio and `reqwest` with rustls (no OpenSSL); a synchronous
//!   `blocking::Honk` with the feature `blocking`.
//! - Every send carries an `Idempotency-Key` (yours, or a UUIDv7) that is reused on every
//!   retry, so a retry never creates a duplicate.
//! - Retries only network errors, timeouts, 429 and 5xx, with exponential backoff and full
//!   jitter, never sooner than `Retry-After`, all under a total deadline (30 s). Redirects are
//!   reported, never followed.
//! - Local validation of everything cheap (lengths, https-only URLs, metadata, the 16 KiB body),
//!   with every invalid field reported at once. [`HonkBuilder::validate`]`(false)` leaves it to
//!   the server.
//!
//! Keep the ingestion key (`honk_…`) on the server: never ship it inside a client app.
//!
//! # Quick start
//!
//! ```no_run
//! use honk_me::{Honk, Message, Severity};
//!
//! #[tokio::main]
//! async fn main() -> honk_me::Result<()> {
//!     let honk = Honk::from_env()?; // HONK_URL, HONK_KEY
//!
//!     // The Honk scale: light, beep, loud, long, blast.
//!     honk.beep("Backup finished", "nightly pg_dump took 42 s").await?;
//!
//!     // Every field, and your own idempotency key.
//!     let msg = Message::new("Ana asked for a quote: 3 rooms, 2 bathrooms")
//!         .title("New quote request")
//!         .severity(Severity::LOUD)
//!         .group_key("requests/4812")
//!         .url("https://shop.example.com/admin/requests/4812")
//!         .metadata("rooms", 3);
//!     let accepted = honk.send(&msg, "request-4812").await?;
//!     println!("stored as {}", accepted.id);
//!
//!     // Incidents: a problem opens one for its group, a recovery closes it.
//!     honk.problem("db/backup", "Backup failed", "pg_dump exited with 1").await?;
//!     honk.recovery("db/backup", "Backup OK", "pg_dump finished").await?;
//!     Ok(())
//! }
//! ```
//!
//! # Errors
//!
//! Every failure is an [`Error`]: `Validation` (fix the message), `Auth`, `Quota`, `Conflict`,
//! `Network`, `Timeout`, `Server` or `Http`, each with a [`Failure`] holding the details.
//! [`Error::is_retryable`] tells whether sending again later (with the same idempotency key)
//! may succeed.
//!
//! # Features
//!
//! - `blocking`: `blocking::Honk`, a synchronous client on a private Tokio runtime.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms, unreachable_pub)]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "blocking")]
#[cfg_attr(docsrs, doc(cfg(feature = "blocking")))]
pub mod blocking;
mod client;
mod error;
mod message;
mod time;
mod uuid;
mod validate;

pub use client::{Honk, HonkBuilder, PendingSend, VERSION};
pub use error::{Error, Failure, FieldError, Result};
pub use message::{
    Accepted, Category, Defaults, EventType, Message, MetadataValue, ParseEnumError, Priority,
    Severity,
};
pub use uuid::new_idempotency_key;
pub use validate::{
    MAX_BODY_BYTES, MAX_CHANNEL, MAX_ENVIRONMENT, MAX_GROUP_KEY, MAX_MESSAGE_BYTES,
    MAX_METADATA_KEYS, MAX_METADATA_STRING, MAX_SOURCE, MAX_TITLE, MAX_TTL_SECONDS, MAX_URL_BYTES,
    MIN_TTL_SECONDS, encode_message,
};
