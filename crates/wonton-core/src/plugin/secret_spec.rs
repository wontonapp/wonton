//! 插件密钥的清单声明（`x-wonton.secrets`，规范"插件密钥"）：每项是核心生成的随机值
//! （`generate`）、插件运行中生成、交给核心保管的文件（`collect`）或主人自己填的值（`input`），
//! 恰好一种；注入与收存在 [`super::secrets`]。`show_owner` 声明主人可在 App 里查看一项生成的密钥。

use serde::{Deserialize, Serialize};

use super::manifest::{LocalizedText, ManifestError, require_locales};

/// 一项插件密钥：核心生成（`generate`）或插件运行中生成、交给核心保管（`collect`），恰好一种。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawSecret", into = "RawSecret")]
pub enum SecretSpec {
    Generate(GenerateSecret),
    Collect(CollectSecret),
    Input(InputSecret),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateSecret {
    /// 随机字节数。
    #[serde(default = "default_secret_bytes")]
    pub bytes: usize,
    #[serde(default)]
    pub encoding: SecretEncoding,
    /// 主人在 App 里查看这一项时的名字。清单写在条目级 `show_owner`，解析时挪到这里；
    /// 序列化时跳过（[`RawSecret`] 再把它放回条目级），往返后写法不变。
    #[serde(skip)]
    pub view: Option<ShowOwner>,
}

/// 生成的密钥的文本格式，和 `openssl rand -hex`、`openssl rand -base64` 相同。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecretEncoding {
    #[default]
    Hex,
    Base64,
}

/// 插件在 `service` 的容器里写下的文件 `path`（容器里的绝对路径）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectSecret {
    pub service: String,
    pub path: String,
}

/// 主人在 App 里查看一项生成的密钥：`show_owner`，`label` 是 App 里这一行的名字。只能加在
/// `generate` 项上——`collect` 项是插件交给核心的凭据，不给人看。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowOwner {
    pub label: LocalizedText,
}

/// 主人在安装、升级时自己填的一项密钥（ADR-0064）：值是用户在别的服务那里拿到的凭据，不返回、只显示
/// 填没填。`url` 是去哪里拿；`required` 缺了装不上。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputSecret {
    /// 输入框的标题，写法同清单的 `title`。
    pub title: LocalizedText,
    /// 一句补充，如免费额度、要开哪些权限。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<LocalizedText>,
    /// 去哪里拿这个密钥，只接受 `https://`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// 必填，默认是。
    #[serde(default = "default_required")]
    pub required: bool,
}

fn default_required() -> bool {
    true
}

/// `https://` 开头、带主机、没有空白。
fn https_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !host.is_empty() && !url.chars().any(char::is_whitespace)
}

/// 生成的密钥最多这么多随机字节。
pub const MAX_SECRET_BYTES: usize = 1024;

fn default_secret_bytes() -> usize {
    32
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSecret {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generate: Option<GenerateSecret>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    collect: Option<CollectSecret>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input: Option<InputSecret>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show_owner: Option<ShowOwner>,
}

impl TryFrom<RawSecret> for SecretSpec {
    type Error = String;

    fn try_from(raw: RawSecret) -> Result<Self, Self::Error> {
        match (raw.generate, raw.collect, raw.input, raw.show_owner) {
            (Some(mut generate), None, None, show_owner) => {
                generate.view = show_owner;
                Ok(Self::Generate(generate))
            }
            (None, Some(_), None, Some(_)) => {
                Err("collect 项是插件交给核心的凭据，不给人看，不能加 show_owner".to_owned())
            }
            (None, Some(collect), None, None) => Ok(Self::Collect(collect)),
            (None, None, Some(_), Some(_)) => {
                Err("input 项的值不返回，不能加 show_owner".to_owned())
            }
            (None, None, Some(input), None) => Ok(Self::Input(input)),
            _ => Err("每项密钥必须恰好写 generate、collect 或 input 之一".to_owned()),
        }
    }
}

impl From<SecretSpec> for RawSecret {
    fn from(spec: SecretSpec) -> Self {
        match spec {
            SecretSpec::Generate(generate) => Self {
                show_owner: generate.view,
                generate: Some(GenerateSecret {
                    bytes: generate.bytes,
                    encoding: generate.encoding,
                    view: None,
                }),
                collect: None,
                input: None,
            },
            SecretSpec::Collect(collect) => Self {
                show_owner: None,
                generate: None,
                collect: Some(collect),
                input: None,
            },
            SecretSpec::Input(input) => Self {
                show_owner: None,
                generate: None,
                collect: None,
                input: Some(input),
            },
        }
    }
}

/// 密钥名：小写字母开头，由小写字母、数字和 `_` 组成；注入为 `WONTON_SECRET_<名字大写>`。
pub fn is_secret_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// `Manifest::validate` 里密钥部分的规则；serde 表达不了的在这里。
pub(super) fn validate_secret(name: &str, spec: &SecretSpec) -> Result<(), ManifestError> {
    let invalid =
        |message: String| Err(ManifestError::Invalid(format!("secrets.{name}：{message}")));
    if !is_secret_name(name) {
        return invalid("名字须以小写字母开头，只含小写字母、数字和 _".to_owned());
    }
    match spec {
        SecretSpec::Generate(generate) => {
            if !(1..=MAX_SECRET_BYTES).contains(&generate.bytes) {
                return invalid(format!("generate.bytes 须在 1 到 {MAX_SECRET_BYTES} 之间"));
            }
            // label 原样进 App 的插件详情页，和 title 一样不能是空的。
            if let Some(view) = &generate.view {
                require_locales("show_owner.label", &view.label)
                    .map_err(|e| ManifestError::Invalid(format!("secrets.{name}：{e}")))?;
            }
            Ok(())
        }
        SecretSpec::Collect(collect)
            if !collect.path.starts_with('/') || collect.path.split('/').any(|p| p == "..") =>
        {
            invalid("collect.path 须是容器里的绝对路径，不能含 ..".to_owned())
        }
        SecretSpec::Input(input) => {
            require_locales("input.title", &input.title)
                .map_err(|e| ManifestError::Invalid(format!("secrets.{name}：{e}")))?;
            if let Some(description) = &input.description {
                require_locales("input.description", description)
                    .map_err(|e| ManifestError::Invalid(format!("secrets.{name}：{e}")))?;
            }
            match &input.url {
                Some(url) if !https_url(url) => invalid("input.url 须是 https:// 地址".to_owned()),
                _ => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::{Manifest, PluginPackage};

    /// 最小可用清单，`secrets` 段由各测试替换。
    fn manifest_with(secrets: &str) -> Result<Manifest, ManifestError> {
        let source = format!(
            "name: demo\n\
             services:\n\
             \x20 app:\n\
             \x20   image: traefik/whoami:v1.11\n\
             x-wonton:\n\
             \x20 spec: '0.4'\n\
             \x20 id: app.secrets.demo\n\
             \x20 version: 0.1.0\n\
             \x20 kind: app\n\
             \x20 title: {{ zh: 密钥, en: Secrets }}\n\
             \x20 description: {{ zh: 测试用, en: For tests }}\n\
             \x20 permissions: []\n\
             \x20 secrets:\n{secrets}"
        );
        PluginPackage::manifest_of(
            &source,
            &super::super::languages::Languages::from_header(None),
        )
        .map_err(|e| ManifestError::Invalid(e.to_string()))
    }

    #[test]
    fn show_owner_lands_on_the_generate_item() {
        let source = "    admin_password:\n\
                      \x20     generate: { bytes: 32 }\n\
                      \x20     show_owner: { label: { zh: 管理员密码, en: Admin password } }";
        let manifest = manifest_with(source).unwrap();
        let Some(SecretSpec::Generate(generate)) = manifest.secrets.get("admin_password") else {
            panic!("应有 generate 项");
        };
        assert_eq!(
            generate.view.as_ref().unwrap().label.get("zh").unwrap(),
            "管理员密码"
        );
        // 序列化回到条目级，写法不变。
        let raw = serde_json::to_value(manifest.secrets.get("admin_password").unwrap()).unwrap();
        assert_eq!(
            raw["show_owner"]["label"]["en"], "Admin password",
            "show_owner 在条目级，不嵌进 generate"
        );
        assert!(raw.get("generate").unwrap().get("show_owner").is_none());
    }

    #[test]
    fn collect_item_rejects_show_owner() {
        let source = "    recovery_key:\n\
                      \x20     collect: { service: app, path: /tmp/key }\n\
                      \x20     show_owner: { label: { zh: 恢复密钥, en: Recovery key } }";
        let error = manifest_with(source).unwrap_err();
        assert!(error.to_string().contains("不能加 show_owner"), "{error}");
    }

    #[test]
    fn show_owner_needs_a_generate_item() {
        let source = "    orphan:\n\
                      \x20     show_owner: { label: { zh: 孤儿, en: Orphan } }";
        let error = manifest_with(source).unwrap_err();
        assert!(error.to_string().contains("恰好写 generate"), "{error}");
    }

    #[test]
    fn label_must_have_text() {
        let source = "    admin_password:\n\
                      \x20     generate: {}\n\
                      \x20     show_owner: { label: { zh: \"  \" } }";
        let error = manifest_with(source).unwrap_err();
        assert!(
            error.to_string().contains("secrets.admin_password"),
            "{error}"
        );
    }
}
