//! 局域网入口能不能用（ADR-0021“局域网直连”、ADR-0089 第 4 节）：每次局域网检测之后记下选中的地址和
//! `*.lan.<后缀>` 的 A 记录对不对，地址变了就重新对账（只重建局域网转发）；打开局域网入口前当场再查一次记录。
//!
//! 记录由主人在自己的 DNS 服务商那里加，核心只查不写。查用的是签发时查 CAA 的同一组解析器。

use std::net::Ipv4Addr;

use wonton_protocol::{ClientsFailure, LanRecordState};

use super::apply::{self, ApplyError};
use crate::entry_containers::EntryContainers;
use crate::gateway::Gateway;
pub use crate::plugin::lan::entry_address::{EntryAddress, Refusal};

/// 局域网入口现在的状态，由局域网检测更新。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanState {
    /// 核心起来后还没检测过：局域网转发保持原样，不能新开局域网入口。
    Unknown,
    /// 选不出能开局域网入口的地址：局域网转发删掉。
    Refused(Refusal),
    /// 选中了地址；`record` 是 `*.lan` 记录最近一次查的结果。记录不对时转发照常发布在这个地址上，
    /// 主人改好记录后不用等重建。
    Selected {
        address: EntryAddress,
        record: Record,
    },
}

/// `*.lan.<后缀>` 的 A 记录查出来的样子。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// 还没配后缀，没东西可查。
    Unchecked,
    Ok,
    /// 没有这条记录（NXDOMAIN 或只有别的类型）。
    Missing,
    /// 解析到了别的地址。
    Wrong(Vec<Ipv4Addr>),
    /// 查询本身失败（解析器连不上、超时）。
    Failed(String),
}

impl LanState {
    /// 局域网转发要不要、发布在哪：`None` 是不确定，转发保持原样。
    pub(crate) fn forward(&self) -> Option<Option<&EntryAddress>> {
        match self {
            Self::Unknown => None,
            Self::Refused(_) => Some(None),
            Self::Selected { address, .. } => Some(Some(address)),
        }
    }

    /// 局域网入口能用：地址选好了，记录也对。
    pub(crate) fn usable(&self) -> bool {
        matches!(
            self,
            Self::Selected {
                record: Record::Ok,
                ..
            }
        )
    }

    /// 域名设置页上的 `lan_record`。
    pub(crate) fn record_state(&self) -> LanRecordState {
        match self {
            Self::Selected { record, .. } => match record {
                Record::Ok => LanRecordState::Ok,
                Record::Missing => LanRecordState::Missing,
                Record::Wrong(found) => LanRecordState::Wrong {
                    found: joined(found),
                },
                Record::Unchecked | Record::Failed(_) => LanRecordState::Unknown,
            },
            Self::Unknown | Self::Refused(_) => LanRecordState::Unknown,
        }
    }
}

/// 局域网检测之后：记下新状态；转发要换地址（或要删、要建）时对一次账。
pub(crate) async fn follow(gateway: &Gateway, picked: Result<EntryAddress, Refusal>) {
    let Some(entry) = &gateway.entry else {
        return;
    };
    let previous = entry.lan();
    let state = match picked {
        Err(refusal) => LanState::Refused(refusal),
        Ok(address) => {
            // 地址没变、记录对过就不再查：ADR-0089 只要求打开时和地址变化时查；不对的每次检测都查，改好就恢复。
            let record = match &previous {
                LanState::Selected {
                    address: old,
                    record: Record::Ok,
                } if *old == address => Record::Ok,
                _ => check_record(gateway, address.address).await,
            };
            LanState::Selected { address, record }
        }
    };
    if state != previous {
        tracing::info!(?state, "局域网入口的状态");
    }
    let moved = previous.forward() != state.forward();
    entry.set_lan(state);
    if moved
        && lan_open(gateway)
        && let Err(error) = apply::reconcile(gateway).await
    {
        tracing::warn!(%error, "局域网地址变了，重建局域网转发失败，入口的对账循环会重试");
    }
}

/// 打开局域网入口前：地址要选得出，记录要当场查对。调用方持有入口的串行锁。
pub(crate) async fn check_open(
    gateway: &Gateway,
    entry: &EntryContainers,
    suffix: &str,
) -> Result<(), ApplyError> {
    let address = match entry.lan() {
        LanState::Unknown => {
            return Err(apply::error(
                ClientsFailure::LanUnavailable,
                "clients-switch-lan",
            ));
        }
        LanState::Refused(refusal) => return Err(refused(&refusal)),
        LanState::Selected { address, .. } => address,
    };
    let record = check_record(gateway, address.address).await;
    // 查的时候检测可能换了地址：只在地址还是这个时记下结果。
    if let LanState::Selected { address: now, .. } = entry.lan()
        && now == address
    {
        entry.set_lan(LanState::Selected {
            address,
            record: record.clone(),
        });
    }
    match record {
        Record::Ok => Ok(()),
        other => Err(wrong_record(&other, suffix, address.address)),
    }
}

/// 有插件开着局域网入口（读不了的记录不算：它的开关反正不在模型里）。
fn lan_open(gateway: &Gateway) -> bool {
    gateway.plugins.client_plugins().is_ok_and(|clients| {
        clients
            .iter()
            .any(|client| client.record.clients_entries.lan)
    })
}

/// 查一个随机名字：`*.lan` 是通配记录，随机名字躲开解析器对具体名字的缓存，也只有通配记录答得上。
async fn check_record(gateway: &Gateway, address: Ipv4Addr) -> Record {
    let Some(domain) = &gateway.domain else {
        return Record::Unchecked;
    };
    let (suffix, resolvers) = match domain.lan_lookup() {
        Ok(Some(found)) => found,
        Ok(None) => return Record::Unchecked,
        Err(error) => return Record::Failed(format!("{error:#}")),
    };
    let mut random = [0u8; 6];
    getrandom::fill(&mut random).expect("操作系统随机数源不可用");
    let label: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let name = format!("probe-{label}.lan.{suffix}");
    match crate::entry_dns::a_records(&name, &resolvers).await {
        Ok(found) if found == [address] => Record::Ok,
        Ok(found) if found.is_empty() => Record::Missing,
        Ok(found) => Record::Wrong(found),
        Err(error) => Record::Failed(format!("{error:#}")),
    }
}

fn refused(refusal: &Refusal) -> ApplyError {
    let (key, args) = match refusal {
        Refusal::Selection(reason) => (
            "clients-switch-lan-selection",
            vec![("reason", reason.clone())],
        ),
        Refusal::NotPrivate(addresses) => (
            "clients-switch-lan-not-private",
            vec![("addresses", joined(addresses))],
        ),
        Refusal::WideSubnet(subnet) => (
            "clients-switch-lan-wide-subnet",
            vec![("subnet", subnet.to_string())],
        ),
        Refusal::UnknownSubnet(address) => (
            "clients-switch-lan-unknown-subnet",
            vec![("address", address.to_string())],
        ),
    };
    apply::keyed(ClientsFailure::LanUnavailable, key, args)
}

fn wrong_record(record: &Record, suffix: &str, address: Ipv4Addr) -> ApplyError {
    let wanted = format!("*.lan.{suffix} A {address}");
    let (key, mut args) = match record {
        Record::Wrong(found) => (
            "clients-switch-lan-record-wrong",
            vec![("found", joined(found))],
        ),
        Record::Missing => ("clients-switch-lan-record-missing", Vec::new()),
        Record::Failed(error) => (
            "clients-switch-lan-record-failed",
            vec![("error", error.clone())],
        ),
        Record::Unchecked | Record::Ok => (
            "clients-switch-lan-record-failed",
            vec![("error", String::new())],
        ),
    };
    args.push(("record", wanted));
    apply::keyed(ClientsFailure::LanUnavailable, key, args)
}

fn joined<T: std::fmt::Display>(items: &[T]) -> String {
    items
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
