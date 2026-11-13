//! 主人填写的插件密钥（ADR-0064）：清单声明见 [`super::secret_spec`]，这里管校验、保存、预览和详情里的那一行，
//! 以及改一项时重建引用它的服务。
//!
//! 值只保存在核心的密钥库里、任何响应都不带；“缺哪几项”按密钥名列出，文字由核心按请求语言写好（ADR-0043）。

use std::collections::BTreeMap;

use wonton_protocol::{SecretInput, SecretInputState, SecretValues};

use super::install_error::InstallError;
use super::languages::Languages;
use super::manager::{PluginManager, now_secs};
use super::manifest::{InputSecret, Manifest, PluginId, SecretSpec};
use super::secrets::Secrets;
use crate::l10n;

/// 清单里 `input` 项名到声明。
pub fn specs(manifest: &Manifest) -> impl Iterator<Item = (&str, &InputSecret)> {
    manifest
        .secrets
        .iter()
        .filter_map(|(name, spec)| match spec {
            SecretSpec::Input(input) => Some((name.as_str(), input)),
            _ => None,
        })
}

/// 预览里要主人填的项。`filled` 是已经填过、这次不用再问的名字（升级时沿用）；装了没填的必填项也算在内。
pub fn lines(
    manifest: &Manifest,
    filled: &dyn Fn(&str) -> bool,
    skip_filled: bool,
    langs: &Languages,
) -> Vec<SecretInput> {
    specs(manifest)
        .filter(|(name, _)| !(skip_filled && filled(name)))
        .map(|(name, input)| SecretInput {
            name: name.to_owned(),
            title: langs.pick_owned(&input.title),
            description: input.description.as_ref().map(|d| langs.pick_owned(d)),
            url: input.url.clone(),
            required: input.required,
        })
        .collect()
}

/// 详情里的每一项：填没填、什么时候改的，没有值。
pub fn states(
    manifest: &Manifest,
    secrets: &Secrets,
    changed: &BTreeMap<String, u64>,
    langs: &Languages,
) -> Vec<SecretInputState> {
    specs(manifest)
        .map(|(name, input)| SecretInputState {
            name: name.to_owned(),
            title: langs.pick_owned(&input.title),
            required: input.required,
            filled: secrets.contains_key(name),
            changed_at_ms: changed.get(name).copied(),
        })
        .collect()
}

/// 清单里必填、还没填的项名（按名字排）。插件照样装上，状态是“待配置”：MCP 工具报未就绪并点名，网页照常。
pub fn missing_required(manifest: &Manifest, secrets: &Secrets) -> Vec<String> {
    specs(manifest)
        .filter(|(name, input)| input.required && !secrets.contains_key(*name))
        .map(|(name, _)| name.to_owned())
        .collect()
}

/// 校验请求里带的密钥表：不认识的项算输入不合法，报错列出名字。缺了的必填项不再拦安装：主人可以装好再填，
/// 在那之前插件是“待配置”。
pub fn check(
    manifest: &Manifest,
    values: &SecretValues,
    langs: &Languages,
) -> Result<(), InstallError> {
    let known: BTreeMap<&str, &InputSecret> = specs(manifest).collect();
    let unknown: Vec<&str> = values
        .keys()
        .map(String::as_str)
        .filter(|name| !known.contains_key(name))
        .collect();
    if !unknown.is_empty() {
        return Err(InstallError::Invalid(l10n::text(
            langs,
            "plugin-secret-unknown",
            &[("names", l10n::list(langs, &unknown).into())],
        )));
    }
    Ok(())
}

/// 把一项写进密钥库。`value` 为空是清空可选项；必填项清空报错。返回有没有变化。
pub fn store(
    manifest: &Manifest,
    secrets: &mut Secrets,
    name: &str,
    value: Option<&str>,
    langs: &Languages,
) -> Result<bool, InstallError> {
    let Some(input) = manifest.secrets.get(name) else {
        return Err(InstallError::Invalid(l10n::text(
            langs,
            "plugin-secret-unknown",
            &[("names", l10n::list(langs, &[name]).into())],
        )));
    };
    let SecretSpec::Input(input) = input else {
        return Err(InstallError::Invalid(l10n::text(
            langs,
            "plugin-secret-not-input",
            &[("name", name.into())],
        )));
    };
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        Some(text) => {
            let bytes = text.as_bytes().to_vec();
            if secrets.get(name) == Some(&bytes) {
                return Ok(false);
            }
            secrets.insert(name.to_owned(), bytes);
            Ok(true)
        }
        None if input.required => Err(InstallError::Invalid(l10n::text(
            langs,
            "plugin-secret-required-clear",
            &[("name", name.into())],
        ))),
        None => Ok(secrets.remove(name).is_some()),
    }
}

impl PluginManager {
    /// 这个插件必填、还没填的密钥名；`manifest` 是调用方手上已经有的清单，没有就按记录读。读不出来按没填算。
    pub fn unfilled_secrets(&self, id: &PluginId, manifest: Option<&Manifest>) -> Vec<String> {
        let manifest = match manifest {
            Some(manifest) => manifest.clone(),
            None => match self.listed(id) {
                Ok(listed) => listed.manifest.clone(),
                Err(_) => return Vec::new(),
            },
        };
        let filled = self
            .store
            .secrets()
            .load(id)
            .ok()
            .flatten()
            .unwrap_or_default();
        missing_required(&manifest, &filled)
    }

    /// 装好的插件每项要填的密钥：填没填、什么时候改的，不带值。
    pub(super) fn installed_input_states(
        &self,
        id: &PluginId,
        manifest: &Manifest,
        langs: &Languages,
    ) -> Vec<SecretInputState> {
        let store = self.store.secrets();
        let filled = store.load(id).ok().flatten().unwrap_or_default();
        let changed = store.changed(id).unwrap_or_default();
        states(manifest, &filled, &changed, langs)
    }

    /// 校验并保存主人填的密钥，记下改动时间。`filled` 是升级时装着的密钥（填过的沿用）。
    /// 结果里是真正变了的名字。
    pub(crate) fn store_input_values(
        &self,
        manifest: &Manifest,
        values: &wonton_protocol::SecretValues,
        filled: Option<&Secrets>,
        langs: &Languages,
    ) -> Result<Vec<String>, InstallError> {
        super::install_entry::check_runtime(manifest, langs)?;
        let store = self.store.secrets();
        let mut secrets = match filled {
            Some(secrets) => secrets.clone(),
            None => store.load(&manifest.id)?.unwrap_or_else(Secrets::new),
        };
        super::secret_input::check(manifest, values, langs)?;
        let mut changed = store.changed(&manifest.id)?;
        let mut written = Vec::new();
        let now = now_secs() * 1000;
        for (name, value) in values {
            if super::secret_input::store(
                manifest,
                &mut secrets,
                name,
                Some(value.expose()),
                langs,
            )? {
                changed.insert(name.clone(), now);
                written.push(name.clone());
            }
        }
        if !written.is_empty() {
            store.save_changed(&manifest.id, &secrets, &changed)?;
        }
        Ok(written)
    }

    /// 卡片要主人填的密钥：装的时候全都列，升级只列还没填的（已经填过的沿用）。
    pub(crate) fn secret_inputs(
        &self,
        manifest: &Manifest,
        filled: Option<&Secrets>,
        langs: &Languages,
    ) -> Result<Vec<wonton_protocol::SecretInput>, InstallError> {
        let stored = match filled {
            Some(secrets) => secrets.clone(),
            None => self
                .store
                .secrets()
                .load(&manifest.id)?
                .unwrap_or_else(Secrets::new),
        };
        let is_filled = |name: &str| stored.contains_key(name);
        Ok(super::secret_input::lines(
            manifest,
            &is_filled,
            filled.is_some(),
            langs,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn langs() -> Languages {
        Languages::from_header(None)
    }

    fn manifest(secrets: &str) -> Manifest {
        let source = format!(
            "name: demo\n\
             services:\n\
             \x20 app:\n\
             \x20   image: traefik/whoami:v1.11\n\
             x-wonton:\n\
             \x20 spec: '0.8'\n\
             \x20 id: app.secrets.input\n\
             \x20 version: 0.1.0\n\
             \x20 kind: app\n\
             \x20 title: {{ zh: 密钥, en: Secrets }}\n\
             \x20 description: {{ zh: 测试用, en: For tests }}\n\
             \x20 permissions: []\n\
             \x20 secrets:\n{secrets}"
        );
        super::super::package::PluginPackage::manifest_of(&source, &langs()).unwrap()
    }

    const BOTH: &str = concat!(
        "    brave_key:\n",
        "      input:\n",
        "        title: { zh: Brave 密钥, en: Brave key }\n",
        "        description: { zh: 每月 2000 次, en: \"2,000 a month\" }\n",
        "        url: https://example.com/keys\n",
        "    hello:\n",
        "      input:\n",
        "        title: { zh: 招呼, en: Greeting }\n",
        "        required: false\n",
    );

    #[test]
    fn preview_lines_follow_the_language_and_skip_filled() {
        let manifest = manifest(BOTH);
        let quiet = langs();
        let plain = lines(&manifest, &|_| false, false, &quiet);
        assert_eq!(plain.len(), 2);
        assert_eq!(plain[0].name, "brave_key");
        assert!(plain[0].required);
        assert_eq!(plain[0].url.as_deref(), Some("https://example.com/keys"));
        assert!(!plain[1].required);
        let filled = |name: &str| name == "brave_key";
        let upgrade = lines(&manifest, &filled, true, &quiet);
        assert_eq!(
            upgrade.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            ["hello"],
            "升级只问还没填的"
        );
    }

    #[test]
    fn check_takes_unknown_names_and_empty_required_ones() {
        let manifest = manifest(BOTH);
        let quiet = langs();
        let empty = SecretValues::new();
        assert!(
            check(&manifest, &empty, &quiet).is_ok(),
            "必填项缺了不再拦安装"
        );
        let unknown =
            SecretValues::from([("nope".to_owned(), wonton_protocol::SecretString::new("x"))]);
        assert!(check(&manifest, &unknown, &quiet).is_err());
    }

    #[test]
    fn missing_required_lists_unfilled_required_names() {
        let manifest = manifest(BOTH);
        let mut secrets = Secrets::new();
        assert_eq!(missing_required(&manifest, &secrets), ["brave_key"]);
        secrets.insert("brave_key".into(), b"s3cret".to_vec());
        assert!(missing_required(&manifest, &secrets).is_empty());
    }

    #[test]
    fn store_takes_and_clears_only_optional() {
        let manifest = manifest(BOTH);
        let quiet = langs();
        let mut secrets = Secrets::new();
        assert!(
            store(
                &manifest,
                &mut secrets,
                "brave_key",
                Some(" s3cret "),
                &quiet
            )
            .unwrap()
        );
        assert_eq!(secrets["brave_key"], b"s3cret");
        assert!(!store(&manifest, &mut secrets, "brave_key", Some("s3cret"), &quiet).unwrap());
        assert!(
            store(&manifest, &mut secrets, "brave_key", None, &quiet).is_err(),
            "必填项不能清空"
        );
        assert!(store(&manifest, &mut secrets, "hello", Some("hi"), &quiet).unwrap());
        assert!(store(&manifest, &mut secrets, "hello", None, &quiet).unwrap());
        assert!(!secrets.contains_key("hello"));
        assert!(store(&manifest, &mut secrets, "nope", Some("x"), &quiet).is_err());
    }

    #[test]
    fn states_show_filled_and_time_without_the_value() {
        let manifest = manifest(BOTH);
        let quiet = langs();
        let mut secrets = Secrets::new();
        secrets.insert("brave_key".into(), b"s3cret".to_vec());
        let changed = BTreeMap::from([("brave_key".to_owned(), 7u64)]);
        let states = states(&manifest, &secrets, &changed, &quiet);
        assert!(states[0].filled);
        assert_eq!(states[0].changed_at_ms, Some(7));
        assert!(!states[1].filled);
        assert_eq!(states[1].changed_at_ms, None);
    }
}

#[cfg(test)]
mod card_tests {
    use wonton_protocol::ConfirmationCard;

    /// 契约：卡片带要填的项，不带值；旧核心没有这个字段也照常解析。
    #[test]
    fn a_card_lists_the_inputs_without_values() {
        let card: ConfirmationCard = serde_json::from_value(serde_json::json!({
            "id": "abc",
            "state": {"type": "pending"},
            "title": "Hermes 想安装 Brave",
            "lines": [],
            "choices": [{"answer": "approve", "label": "安装"}],
            "expires_at_ms": 1,
            "secret_inputs": [{
                "name": "brave_key",
                "title": "Brave Search API key",
                "required": true,
            }],
        }))
        .unwrap();
        assert_eq!(card.secret_inputs[0].name, "brave_key");
        let old: ConfirmationCard = serde_json::from_value(serde_json::json!({
            "id": "abc",
            "state": {"type": "pending"},
            "title": "允许 ls",
            "lines": [],
            "choices": [],
            "expires_at_ms": 1,
        }))
        .unwrap();
        assert!(old.secret_inputs.is_empty());
        assert!(
            !serde_json::to_string(&card).unwrap().contains("\"value\""),
            "值不在卡片里"
        );
    }
}
