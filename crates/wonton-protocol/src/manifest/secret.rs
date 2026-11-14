use serde::{Deserialize, Serialize};

use super::LocalizedText;

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

/// 生成的密钥的文本格式。`hex`、`base64` 和 `openssl rand -hex`、`openssl rand -base64` 相同；
/// `base64url` 是无填充的 base64url（RFC 4648 §5），给只收 URL 安全字符的启动参数用。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecretEncoding {
    #[default]
    Hex,
    Base64,
    Base64Url,
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
