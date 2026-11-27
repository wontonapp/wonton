use iroh::RelayUrl;

use super::n0::n0_relays;
use super::parse::relay_from_item;
use super::relay::{Relay, RelayView};
use super::url::{canonical_relay, canonical_url};

/// 读中继表的环境变量。
pub const RELAYS_VAR: &str = "WONTON_RELAYS";

/// 关键字：iroh 内置的 n0 公共中继那一组。
const N0: &str = "n0";

/// 中继表：按档位排好的中继，前面的档优先。同一地址只出现一次，后面的项盖过前面的（ADR-0048）。
///
/// `remove` 是覆盖层的删除标记（`!https://…`）：[`overlay`](Self::overlay) 先把这些地址从底表删掉，
/// 再放自己这一层的项。主人配置里有内置项想删掉时用。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelayTable {
    relays: Vec<Relay>,
    remove: Vec<RelayUrl>,
}

impl RelayTable {
    /// 不设 [`RELAYS_VAR`] 时的表：n0 的公共中继一档。
    pub fn builtin_n0() -> Self {
        Self {
            relays: n0_relays(0).collect(),
            remove: Vec::new(),
        }
    }

    /// 从配置解析出来的表项组表；地址规范成 [`canonical_url`] 的一种写法。
    pub fn from_relays(relays: Vec<Relay>) -> Self {
        let mut relays: Vec<_> = relays.into_iter().map(canonical_relay).collect();
        relays.sort_by_key(|relay| relay.tier);
        Self {
            relays,
            remove: Vec::new(),
        }
    }

    /// 从连接信息或配对码里带来的表项组表。表项要先用 [`RelayView::validate`] 校验过。
    pub fn from_views(relays: Vec<RelayView>) -> Self {
        Self::from_relays(relays.into_iter().map(Relay::from).collect())
    }

    /// 记录要删掉的地址。合并时先删，再放自己的项。
    pub fn with_remove(mut self, remove: Vec<RelayUrl>) -> Self {
        for url in remove {
            let url = canonical_url(url);
            if !self.remove.contains(&url) {
                self.remove.push(url);
            }
        }
        self
    }

    /// 要删掉的地址。
    pub fn remove(&self) -> &[RelayUrl] {
        &self.remove
    }

    /// 解析 [`RELAYS_VAR`] 的值。写得不合法时说清是哪一项、为什么，不静默跳过。
    ///
    /// `!https://relay.example.org` 是删除标记：把这一项从更早的来源里删掉。`#7843` 是 QUIC 端口。
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut relays: Vec<Relay> = Vec::new();
        let mut remove: Vec<RelayUrl> = Vec::new();
        for (index, tier_text) in text.split(';').enumerate() {
            let tier = u8::try_from(index).map_err(|_| "档位太多".to_owned())?;
            for item in tier_text.split(',') {
                let item = item.trim();
                if item.is_empty() {
                    return Err(format!("第 {} 档里有空的中继项", index + 1));
                }
                if item == N0 {
                    relays.extend(n0_relays(tier));
                } else if let Some(url) = item.strip_prefix('!') {
                    if url == N0 {
                        remove.extend(n0_relays(0).map(|relay| relay.url));
                    } else {
                        remove.push(relay_from_item(url, tier)?.url);
                    }
                } else {
                    relays.push(relay_from_item(item, tier)?);
                }
            }
        }
        // 同一地址后面的项盖过前面的：档位和令牌以最后一次写的为准。
        let mut unique: Vec<Relay> = Vec::new();
        for relay in relays {
            if let Some(taken) = unique.iter().position(|old| old.url == relay.url) {
                unique.remove(taken);
            }
            unique.push(relay);
        }
        Ok(Self::from_relays(unique).with_remove(remove))
    }

    /// 全部的表项，档位在前。
    pub fn relays(&self) -> &[Relay] {
        &self.relays
    }

    /// 报给 App 的写法（配对码和 `/_wonton/api/tunnel`），档位在前。
    pub fn views(&self) -> Vec<RelayView> {
        self.relays.iter().map(Relay::view).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.relays.is_empty()
    }

    /// 档位的数量。
    pub fn tier_count(&self) -> usize {
        self.relays
            .last()
            .map_or(0, |relay| relay.tier as usize + 1)
    }

    /// 一档里的全部中继。没有这一档时是空的。
    pub fn tier(&self, tier: u8) -> &[Relay] {
        let start = self.relays.partition_point(|relay| relay.tier < tier);
        let end = self.relays.partition_point(|relay| relay.tier <= tier);
        &self.relays[start..end]
    }

    /// 把自己这一层盖在 `base` 上：先删 `remove` 里的地址和同 URL 的旧项，再追加自己的项。结果按
    /// 档位稳定排序，不是就地把项挪到新位置。
    pub fn overlay(&self, base: &RelayTable) -> RelayTable {
        let mut relays: Vec<Relay> = base
            .relays
            .iter()
            .filter(|relay| {
                !self.remove.contains(&relay.url)
                    && !self.relays.iter().any(|own| own.url == relay.url)
            })
            .cloned()
            .collect();
        relays.extend(self.relays.iter().cloned().map(canonical_relay));
        let mut unique: Vec<Relay> = Vec::new();
        for relay in relays {
            if let Some(taken) = unique.iter().position(|old| old.url == relay.url) {
                unique.remove(taken);
            }
            unique.push(relay);
        }
        let mut remove = base.remove.clone();
        for url in &self.remove {
            if !remove.contains(url) {
                remove.push(url.clone());
            }
        }
        RelayTable {
            relays: RelayTable::from_relays(unique).relays,
            remove,
        }
    }

    /// 删掉这些地址（连同它们自己的项）。和 [`overlay`](Self::overlay) 的删除标记一样，合并时还盖底表。
    pub fn without_urls(&self, urls: &[RelayUrl]) -> RelayTable {
        let relays = self
            .relays
            .iter()
            .filter(|relay| !urls.contains(&relay.url))
            .cloned()
            .collect();
        let mut remove = self.remove.clone();
        for url in urls {
            if !remove.contains(url) {
                remove.push(url.clone());
            }
        }
        RelayTable { relays, remove }
    }

    /// 需要 n0 兜底时把 n0 放在最后一档。已经显式写了 n0（或者显式删了某台）就不重复加。
    pub fn with_n0_last(&self, enabled: bool) -> RelayTable {
        if !enabled {
            return self.clone();
        }
        let n0: Vec<Relay> = n0_relays(0)
            .filter(|relay| {
                !self.remove.contains(&relay.url)
                    && !self.relays.iter().any(|own| own.url == relay.url)
            })
            .collect();
        if n0.is_empty() {
            return self.clone();
        }
        let tier = self
            .relays
            .last()
            .map_or(0, |relay| relay.tier.saturating_add(1));
        let mut relays = self.relays.clone();
        relays.extend(n0.into_iter().map(|relay| Relay { tier, ..relay }));
        RelayTable {
            relays: RelayTable::from_relays(relays).relays,
            remove: self.remove.clone(),
        }
    }
}
