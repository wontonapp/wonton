use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use subtle::ConstantTimeEq;

#[derive(Clone, Serialize, Deserialize)]
pub struct AcmeDns {
    pub username: String,
    pub password: String,
    pub subdomain: String,
    pub fulldomain: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Caa {
    pub account_uri: String,
    pub effective_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Home {
    pub endpoint: String,
    pub invite_hash: String,
    pub token_hash: String,
    pub acme_dns: AcmeDns,
    pub lan_acme_dns: AcmeDns,
    pub suspended: bool,
    pub deleted: bool,
    pub reason: Option<String>,
    pub caa: Option<Caa>,
    pub pending_caa: Option<Caa>,
    #[serde(default)]
    pub usage_month: u64,
    #[serde(default)]
    pub usage_bytes: u64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Persisted {
    pub homes: BTreeMap<String, Home>,
    pub invites: BTreeMap<String, u64>,
    pub nonces: BTreeMap<String, u64>,
    #[serde(default)]
    pub counters: BTreeMap<String, u64>,
}

pub struct Store {
    pub data: Persisted,
    path: PathBuf,
    // Held for the entire daemon lifetime: two daemons must never mutate the same volume.
    _lock: File,
}

impl Store {
    pub fn open(path: PathBuf) -> Result<Self> {
        let parent = path.parent().context("state path requires parent")?;
        std::fs::create_dir_all(parent)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("lock"))?;
        lock.try_lock_exclusive()
            .context("home service already owns this state")?;
        let data = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("invalid home state")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Persisted::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            data,
            path,
            _lock: lock,
        })
    }

    pub fn save(&self) -> Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        let temporary = self.path.with_extension("pending");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(&self.data)?)?;
        file.sync_all()?;
        std::fs::rename(&temporary, &self.path)?;
        File::open(self.path.parent().context("state parent")?)?.sync_all()?;
        Ok(())
    }

    pub fn socket_path(path: &Path) -> PathBuf {
        path.with_extension("sock")
    }
}

pub fn hash(value: &[u8]) -> String {
    hex(&Sha256::digest(value))
}

pub fn matches(value: &str, expected_hash: &str) -> bool {
    bool::from(
        hash(value.as_bytes())
            .as_bytes()
            .ct_eq(expected_hash.as_bytes()),
    )
}

pub fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 15) as usize] as char);
    }
    result
}

pub fn random_token() -> Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).context("OS randomness unavailable")?;
    Ok(hex(&bytes))
}

pub fn random_label() -> Result<String> {
    const BASE32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut bytes = [0; 8];
    getrandom::fill(&mut bytes).context("OS randomness unavailable")?;
    Ok(bytes
        .iter()
        .map(|b| BASE32[(b & 31) as usize] as char)
        .collect())
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before Unix epoch")
        .as_secs()
}
