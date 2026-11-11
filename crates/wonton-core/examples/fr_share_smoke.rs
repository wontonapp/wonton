//! Isolated ADR-0051 E2E driver. See deploy/dev/core/smoke-sharing.sh.
#[path = "fr_share_smoke/owner.rs"]
mod owner;
#[path = "fr_share_smoke/scenario.rs"]
mod scenario;
#[path = "fr_share_smoke/wire.rs"]
mod wire;

use anyhow::{Context, Result, ensure};
use std::path::Path;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("seed") => seed(Path::new(args.get(2).context("seed requires state path")?)),
        Some("run") => scenario::run().await,
        _ => anyhow::bail!("usage: fr_share_smoke seed <state> | run"),
    }
}

fn seed(state: &Path) -> Result<()> {
    std::fs::create_dir_all(state)?;
    for name in ["pebble", "bad-cert"] {
        let pair = rcgen::generate_simple_self_signed(vec![format!("{name}.test")])?;
        std::fs::write(state.join(format!("{name}.crt")), pair.cert.pem())?;
        std::fs::write(
            state.join(format!("{name}.key")),
            pair.signing_key.serialize_pem(),
        )?;
    }
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)?;
    std::fs::write(state.join("matrix.key"), key.serialize_pem())?;
    std::fs::write(state.join("matrix.pub"), key.public_key_pem())?;
    for server in ["owner", "friend"] {
        std::fs::create_dir_all(state.join(server).join("core"))?;
    }
    let path = state.join("owner/core/friends.json");
    ensure!(!path.exists(), "smoke state must be fresh");
    let friends = wonton_core::friends::Friends::open(path)?;
    friends.accept("@bob:friend.test", Some("Bob"), 1, |_| false)?;
    friends.accept("@carol:external.test", Some("Carol"), 2, |_| false)?;
    Ok(())
}
