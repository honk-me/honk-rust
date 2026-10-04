//! cargo run --example quickstart   (with HONK_URL and HONK_KEY set; sends one message)
use honk_me::{Honk, Priority};

#[tokio::main]
async fn main() -> honk_me::Result<()> {
    let honk = Honk::from_env()?;
    let accepted = honk
        .loud("Disk 91% full", "db-1 /var is at 91%")
        .group_key("disk/db-1/var")
        .priority(Priority::High)
        .metadata("host", "db-1")
        .await?;
    println!(
        "stored as {} (duplicate: {})",
        accepted.id, accepted.duplicate
    );
    Ok(())
}
