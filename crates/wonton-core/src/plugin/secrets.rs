//! 插件密钥（`x-wonton.secrets`）：核心生成的随机值，和插件运行中写下、交给核心保管的文件。
//!
//! 每个插件一个 JSON 文件（0600），在插件记录目录下的 `secrets/` 里，插件的容器访问不到。卸载时保留
//! 数据就一起保留，重装时沿用：数据库密码之类已经写进了数据，换了就对不上。删除数据时一起删除。

use std::collections::BTreeMap;
use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use super::docker::{ContainerFile, Engine, EngineError};
use super::manifest::{GenerateSecret, Manifest, PluginId, SecretEncoding, SecretSpec};
use super::names::Instance;
use super::store::{
    StoreError, base64_map, create_private_dir, read_json, remove_file, write_json,
};

/// 收存的文件最大 64 KiB。
pub const MAX_COLLECT_BYTES: usize = 64 << 10;

/// 一个插件的密钥：名字 → 值。生成的是文本，收存的是文件原样。
pub type Secrets = BTreeMap<String, Vec<u8>>;

#[derive(Serialize, Deserialize)]
struct SecretFile {
    #[serde(with = "base64_map")]
    values: Secrets,
}

pub struct SecretStore {
    dir: PathBuf,
}

impl SecretStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, id: &PluginId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// 读出插件的密钥；从没保存过时返回 `None`。
    pub fn load(&self, id: &PluginId) -> Result<Option<Secrets>, StoreError> {
        Ok(read_json::<SecretFile>(&self.path(id))?.map(|file| file.values))
    }

    pub fn save(&self, id: &PluginId, secrets: &Secrets) -> Result<(), StoreError> {
        create_private_dir(&self.dir)?;
        let file = SecretFile {
            values: secrets.clone(),
        };
        write_json(&self.path(id), &file)
    }

    pub fn remove(&self, id: &PluginId) -> Result<(), StoreError> {
        remove_file(&self.path(id))
    }
}

/// 按声明生成一个值，格式和 `openssl rand -hex <bytes>`、`openssl rand -base64 <bytes>` 相同（base64 不换行）。
pub fn generate(spec: &GenerateSecret) -> String {
    let mut bytes = vec![0u8; spec.bytes];
    getrandom::fill(&mut bytes).expect("操作系统随机数源不可用");
    match spec.encoding {
        SecretEncoding::Hex => bytes.iter().map(|b| format!("{b:02x}")).collect(),
        SecretEncoding::Base64 => STANDARD.encode(&bytes),
    }
}

/// 给清单里还没有值的 `generate` 项生成值，返回是否有新增。已有的值不动：升级时保留，只给新增的项生成。
pub fn generate_missing(manifest: &Manifest, secrets: &mut Secrets) -> bool {
    let mut added = false;
    for (name, spec) in &manifest.secrets {
        if let SecretSpec::Generate(generate_spec) = spec
            && !secrets.contains_key(name)
        {
            secrets.insert(name.clone(), generate(generate_spec).into_bytes());
            added = true;
        }
    }
    added
}

/// 要注入的 `generate` 项：名字 → 文本值。缺了哪一项（密钥文件被删或损坏后重建）就报出它的名字。
pub fn generated_values(
    manifest: &Manifest,
    secrets: &Secrets,
) -> Result<BTreeMap<String, String>, String> {
    manifest
        .secrets
        .iter()
        .filter(|(_, spec)| matches!(spec, SecretSpec::Generate(_)))
        .map(|(name, _)| {
            let value = secrets
                .get(name)
                .ok_or_else(|| format!("插件密钥 {name} 丢失"))?;
            let text = String::from_utf8(value.clone())
                .map_err(|_| format!("插件密钥 {name} 不是文本"))?;
            Ok((name.clone(), text))
        })
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum CollectError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("插件密钥 {name}：容器 {container} 里的 {path} {reason}")]
    Invalid {
        name: String,
        container: String,
        path: String,
        reason: String,
    },
}

/// 从插件的容器里收存 `collect` 项，返回有没有变化。文件还不存在时跳过，下次启动时再收；核心不修改、
/// 不删除插件里的这个文件，也不把它注入回去。
pub async fn collect(
    engine: &Engine,
    instance: &Instance,
    manifest: &Manifest,
    secrets: &mut Secrets,
) -> Result<bool, CollectError> {
    let mut changed = false;
    for (name, spec) in &manifest.secrets {
        let SecretSpec::Collect(spec) = spec else {
            continue;
        };
        let container = instance.container(&manifest.id.slug(), &spec.service);
        let invalid = |reason: &str| CollectError::Invalid {
            name: name.clone(),
            container: container.clone(),
            path: spec.path.clone(),
            reason: reason.to_owned(),
        };
        match engine
            .read_file(&container, &spec.path, MAX_COLLECT_BYTES)
            .await?
        {
            ContainerFile::Regular(data) => {
                if secrets.get(name) != Some(&data) {
                    secrets.insert(name.clone(), data);
                    changed = true;
                }
            }
            ContainerFile::Missing => {
                tracing::info!(plugin = %manifest.id, secret = %name, "插件还没有写下要收存的密钥，下次启动时再收");
            }
            ContainerFile::NotRegular => return Err(invalid("不是普通文件")),
            ContainerFile::TooBig => return Err(invalid("超过 64 KiB")),
        }
    }
    Ok(changed)
}

/// 注入的变量名：`WONTON_SECRET_<名字大写>`。
pub fn secret_var(name: &str) -> String {
    format!("WONTON_SECRET_{}", name.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_values_match_openssl_rand_formats() {
        let spec = |bytes, encoding| GenerateSecret { bytes, encoding };
        let hex = generate(&spec(32, SecretEncoding::Hex));
        assert_eq!(hex.len(), 64);
        assert!(
            hex.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert_ne!(hex, generate(&spec(32, SecretEncoding::Hex)));
        let b64 = generate(&spec(32, SecretEncoding::Base64));
        assert_eq!(STANDARD.decode(&b64).unwrap().len(), 32);
        assert_eq!(
            b64.len(),
            44,
            "和 openssl rand -base64 32 一样带填充、不换行"
        );
        assert_eq!(secret_var("web_key"), "WONTON_SECRET_WEB_KEY");
    }

    #[test]
    fn secrets_survive_reopen_and_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets"));
        let id = PluginId::parse("app.test").unwrap();
        assert_eq!(store.load(&id).unwrap(), None);
        let secrets: Secrets = [("key".to_owned(), vec![0, 1, 0xff])].into();
        store.save(&id, &secrets).unwrap();
        let reopened = SecretStore::new(dir.path().join("secrets"));
        assert_eq!(reopened.load(&id).unwrap(), Some(secrets));
        let mode = |p: PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(dir.path().join("secrets/app.test.json")), 0o600);
        assert_eq!(mode(dir.path().join("secrets")), 0o700);
        reopened.remove(&id).unwrap();
        reopened.remove(&id).unwrap();
        assert_eq!(reopened.load(&id).unwrap(), None);
    }
}
