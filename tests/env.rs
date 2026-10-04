//! `from_env` in its own test binary: environment variables are process-wide.
mod common;

use common::{KEY, Mock};
use honk_me::{Error, Honk, Message};

#[tokio::test]
async fn from_env_reads_the_honk_variables() {
    let mock = Mock::start(vec![]).await;
    // SAFETY: this test binary has a single test, so nothing reads the environment concurrently.
    unsafe {
        std::env::remove_var("HONK_URL");
        std::env::remove_var("HONK_KEY");
    }
    let err = Honk::from_env().unwrap_err();
    assert!(
        matches!(&err, Error::InvalidConfiguration(m) if m.contains("HONK_URL")),
        "{err}"
    );

    unsafe {
        std::env::set_var("HONK_URL", &mock.url);
        std::env::set_var("HONK_KEY", KEY);
        std::env::set_var("HONK_SOURCE", "cron");
        std::env::set_var("HONK_ENVIRONMENT", "production");
        std::env::set_var("HONK_CHANNEL", " ");
    }
    Honk::from_env()
        .unwrap()
        .send(&Message::new("x"), None)
        .await
        .unwrap();
    assert_eq!(
        mock.requests()[0].json(),
        serde_json::json!({ "message": "x", "source": "cron", "environment": "production" })
    );
}
