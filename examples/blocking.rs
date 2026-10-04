//! cargo run --example blocking --features blocking   (with HONK_URL and HONK_KEY set)
use honk_me::blocking::Honk;

fn main() -> honk_me::Result<()> {
    let honk = Honk::from_env()?;
    let accepted = honk
        .beep("Backup finished", "nightly pg_dump took 42 s")
        .send()?;
    println!("stored as {}", accepted.id);
    Ok(())
}
