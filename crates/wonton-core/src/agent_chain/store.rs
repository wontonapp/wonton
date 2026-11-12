//! 链账本落盘：核心数据目录下的一个 JSON 文件，原子替换写入（纪律同 `friends/persistence`）。
//!
//! 只存链本身：身份、层数、上限、计数和状态。会话、唤醒标签和卡片不落盘——重启时所有
//! 连接都断（ADR-0061 第 2 节），进行中的链一律转成 [`AgentChainStatus::Ended`]，这些跟着失效。

use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use serde::{Deserialize, Serialize};
use wonton_protocol::{AgentChainStatus, BotRef, ChainLimitKind, ChainLimits, ConfirmationOrigin};

use super::ChainLedgerError;

#[derive(Serialize, Deserialize)]
struct StoredChains {
    chains: Vec<StoredChain>,
}

/// 一条链的持久形态。字段只增不改名；读回时凭缺失字段直接报损坏，不猜默认值。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct StoredChain {
    pub(super) chain: String,
    /// 链代表的人：最先说话的那个人，之后每一跳都替他说话（ADR-0061 第 10 节）。
    pub(super) user: String,
    pub(super) origin: ConfirmationOrigin,
    /// 链上的 Bot，按登记顺序排列（即层序）。
    pub(super) bots: Vec<ChainBotEntry>,
    /// 这条链的上限：开链时抄当时的全局设置，"再给一份"在此基础上加。
    pub(super) limits: ChainLimits,
    /// 链上所有会话的 `session/prompt` 总数。
    pub(super) rounds: u32,
    /// 经模型网关记账的 token 总数；没经网关的用量记不上，只受轮数上限管。
    pub(super) tokens: u64,
    /// 网关报过的金额合计。任何一笔没报金额就算不出总额，保持 `None` 不猜。
    pub(super) amount: Option<f64>,
    /// 按 Bot 分账，各项相加等于 [`StoredChain::tokens`]。
    pub(super) spend_by_bot: Vec<BotSpend>,
    pub(super) status: AgentChainStatus,
    /// `status` 是 `limit_reached` 时是哪一种上限，其余为 `None`。
    pub(super) limit_kind: Option<ChainLimitKind>,
    /// 进入过 `limit_reached` 的次数；每次进入恰好对应一次"用到上限"。
    pub(super) limit_events: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ChainBotEntry {
    pub(super) bot: BotRef,
    pub(super) layer: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct BotSpend {
    pub(super) bot: BotRef,
    pub(super) tokens: u64,
}

/// 缺文件等于没有链；坏文件报错，绝不拿空数据顶替。
pub(super) fn read(path: &Path) -> Result<Vec<StoredChain>, ChainLedgerError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(ChainLedgerError::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    let corrupt = |message: String| ChainLedgerError::Corrupt {
        path: path.to_owned(),
        message,
    };
    let stored: StoredChains =
        serde_json::from_slice(&bytes).map_err(|error| corrupt(error.to_string()))?;
    Ok(stored.chains)
}

/// 私有临时文件、fsync、原子改名、目录 fsync，同 `friends/persistence`：改名即提交点，
/// 之后的目录同步失败只算持久性警告——报失败会让盘上和内存里已经一致的数据显得分叉。
pub(super) fn write(path: &Path, chains: &[&StoredChain]) -> Result<(), ChainLedgerError> {
    let io = |source| ChainLedgerError::Io {
        path: path.to_owned(),
        source,
    };
    let bytes = serde_json::to_vec_pretty(&StoredChains {
        chains: chains.iter().map(|chain| (*chain).clone()).collect(),
    })
    .map_err(|error| ChainLedgerError::Serialize(error.to_string()))?;
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(io)?;
    let directory = fs::File::open(dir).map_err(io)?;
    let temp = dir.join(format!(
        ".agent-chains-{}.tmp",
        crate::secret::random_token()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .map_err(io)?;
    let result = (|| {
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        use std::os::fd::AsFd;
        if let Err(error) = crate::plugin::store::sync_dir(directory.as_fd()) {
            tracing::warn!(path = %dir.display(), %error, "agent chain ledger directory sync failed; a crash may lose the committed change");
        }
        Ok::<(), std::io::Error>(())
    })();
    if result.is_err() {
        // 失败时不删目标文件：它可能还是上一份完整的账本。
        let _ = fs::remove_file(&temp);
    }
    result.map_err(io)
}
