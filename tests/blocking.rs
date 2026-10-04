//! The synchronous client (feature `blocking`).
#![cfg(feature = "blocking")]

mod common;

use std::time::Duration;

use common::{KEY, Mock, Reply};
use honk_me::blocking::Honk;
use honk_me::{Error, Message, Severity};

fn mock(script: Vec<Reply>) -> (tokio::runtime::Runtime, Mock) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mock = rt.block_on(Mock::start(script));
    (rt, mock)
}

#[test]
fn sends_retries_and_maps_errors_without_a_runtime() {
    let (_rt, mock) = mock(vec![
        Reply::error(503, "unavailable", "busy"),
        Reply::accepted("msg_1"),
        Reply::error(401, "invalid_key", "bad key"),
    ]);
    let honk = Honk::builder()
        .url(&mock.url)
        .key(KEY)
        .backoff(Duration::from_millis(1), Duration::from_millis(5))
        .build_blocking()
        .unwrap();
    let a = honk
        .loud("Disk 91% full", "db-1 /var is at 91%")
        .group_key("disk/db-1")
        .idempotency_key("b-1")
        .send()
        .unwrap();
    assert_eq!(a.id, "msg_1");
    let err = honk
        .send(&Message::new("x").severity(Severity::BLAST), None)
        .unwrap_err();
    assert!(matches!(err, Error::Auth(_)), "{err:?}");

    let reqs = mock.requests();
    assert_eq!(reqs.len(), 3);
    assert_eq!(reqs[0].header("idempotency-key"), "b-1");
    assert_eq!(reqs[1].header("idempotency-key"), "b-1");
    assert_eq!(reqs[1].json()["severity"], "warning");
    assert_eq!(reqs[2].json()["severity"], "critical");
}

#[test]
fn is_shareable_between_threads() {
    let (_rt, mock) = mock(vec![]);
    let honk = Honk::new(&mock.url, KEY).unwrap();
    std::thread::scope(|s| {
        for i in 0..4 {
            let honk = honk.clone();
            s.spawn(move || honk.beep(format!("job {i}"), "done").send().unwrap());
        }
    });
    assert_eq!(mock.count(), 4);
    assert!(Honk::new("", KEY).is_err());
}
