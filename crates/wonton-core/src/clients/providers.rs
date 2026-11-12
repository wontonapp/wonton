//! DNS 服务商预设：核心不认识具体服务商，只把 lego 的代码和它的凭据变量交给设置页（ADR-0089 第 2 节）。
//!
//! 数据是 `data/dns-providers.json`，加一家只改数据。每项的凭据变量取自 `LEGO_IMAGE` 那个版本
//! `providers/dns/<代码>/<代码>.toml` 的 `[Configuration.Credentials]`，`scripts/gen-dns-presets.sh` 核对。

use serde::Deserialize;

const PRESETS: &str = include_str!("../../data/dns-providers.json");

/// 一个预设。C2 的 `wonton_protocol::clients::DnsProviderPreset` 合并后换成它。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Preset {
    /// lego 的 provider 代码，同时是 `providers/dns` 下的目录名，进 `--dns <代码>`。
    /// 字符集受 `CertificateManager` 的校验限制：只有小写字母和数字。
    pub code: String,
    /// 品牌名，不翻译。
    pub name: String,
    /// 这家必填的凭据环境变量，设置页按这个顺序问。
    pub credential_vars: Vec<String>,
    /// 去哪注册、拿凭据。
    pub signup_url: Option<String>,
    /// 默认推的一家。
    #[serde(default)]
    pub recommended: bool,
}

/// 全部预设，顺序就是设置页的顺序。
pub fn presets() -> Vec<Preset> {
    serde_json::from_str(PRESETS).expect("embedded dns-providers.json parses")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `LEGO_IMAGE` 那个版本的 provider 代码清单，`scripts/gen-dns-presets.sh` 生成。
    /// 和 `entry_certificate.rs` 的 `LEGO_IMAGE` 一起升，一起提交。
    const PINNED_LEGO_PROVIDERS: &str = include_str!("../entry_certificate/lego_providers.txt");

    #[test]
    fn presets_parse() {
        let presets = presets();
        assert_eq!(presets.len(), 4);
        let mut codes: Vec<_> = presets.iter().map(|p| p.code.as_str()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), presets.len(), "服务商代码重复");
        for preset in &presets {
            assert!(!preset.name.is_empty(), "{} 没有名字", preset.code);
            assert!(
                !preset.credential_vars.is_empty(),
                "{} 没有凭据变量",
                preset.code
            );
        }
    }

    /// 没有域名的用户照着第一家去 deSEC 注册，它必须是唯一推荐、排第一。
    #[test]
    fn desec_is_recommended_and_first() {
        let presets = presets();
        let desec = &presets[0];
        assert_eq!(desec.code, "desec");
        assert!(desec.recommended);
        assert!(presets[1..].iter().all(|p| !p.recommended));
    }

    #[test]
    fn preset_codes_exist_in_pinned_lego() {
        for preset in presets() {
            assert!(
                PINNED_LEGO_PROVIDERS
                    .lines()
                    .any(|line| line == preset.code),
                "{} 不在固定的 lego 里，先升 LEGO_IMAGE 和清单",
                preset.code
            );
        }
    }

    /// 代码和变量名都原样进 lego 命令行和 dotenv 文件：代码按 `CertificateManager` 的字符集，
    /// 变量名按环境变量字符集，注册页是 https。
    #[test]
    fn credential_vars_charset() {
        for preset in presets() {
            assert!(
                preset
                    .code
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                "{} 的代码字符集不对",
                preset.code
            );
            for var in &preset.credential_vars {
                assert!(
                    var.starts_with(|c: char| c.is_ascii_uppercase())
                        && var
                            .bytes()
                            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_'),
                    "{} 的凭据变量 {} 不是环境变量名",
                    preset.code,
                    var
                );
            }
            if let Some(url) = &preset.signup_url {
                assert!(
                    url.starts_with("https://"),
                    "{} 的注册页不是 https",
                    preset.code
                );
            }
        }
    }
}
