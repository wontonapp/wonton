use matrix_sdk::encryption::verification::{CancelInfo, Emoji, SasState, VerificationRequestState};
use matrix_sdk::ruma::events::key::verification::cancel::CancelCode;
use wonton_protocol::{VerificationCancel, VerificationEmoji, VerificationStage};

use super::Snapshot;
use crate::{Error, Result};

// Matrix SAS table: https://spec.matrix.org/latest/client-server-api/#sas-method-emoji
// Canonical indices: https://github.com/matrix-org/matrix-spec/blob/main/data-definitions/sas-emoji.json
// Includes the variation selectors used by matrix-sdk-crypto 0.19.1's spec-derived table.
const SAS_EMOJI: [&str; 64] = [
    "🐶", "🐱", "🦁", "🐎", "🦄", "🐷", "🐘", "🐰", "🐼", "🐓", "🐧", "🐢", "🐟", "🐙", "🦋", "🌷",
    "🌳", "🌵", "🍄", "🌏", "🌙", "☁️", "🔥", "🍌", "🍎", "🍓", "🌽", "🍕", "🎂", "❤️", "😀", "🤖",
    "🎩", "👓", "🔧", "🎅", "👍", "☂️", "⌛", "⏰", "🎁", "💡", "📕", "✏️", "📎", "✂️", "🔒", "🔑",
    "🔨", "☎️", "🏁", "🚂", "🚲", "✈️", "🚀", "🏆", "⚽", "🎸", "🎺", "🔔", "⚓", "🎧", "📁", "📌",
];

pub(super) fn remember_peer(
    state: &VerificationRequestState,
    snapshot: &mut Snapshot,
) -> Result<()> {
    match state {
        VerificationRequestState::Requested {
            other_device_data, ..
        }
        | VerificationRequestState::Ready {
            other_device_data, ..
        } => {
            snapshot.other_device_id = other_device_data.device_id().to_string();
            snapshot.other_device_name = other_device_data.display_name().map(str::to_owned);
        }
        VerificationRequestState::Transitioned { verification } => {
            let sas = verification
                .clone()
                .sas()
                .ok_or_else(|| Error::Upstream("收到非 SAS 的设备确认流程".to_owned()))?;
            snapshot.other_device_id = sas.other_device().device_id().to_string();
            snapshot.other_device_name = sas.other_device().display_name().map(str::to_owned);
            snapshot.sas = Some(sas);
        }
        _ => (),
    }
    Ok(())
}

pub(super) fn stage(
    request: &VerificationRequestState,
    snapshot: &Snapshot,
    started_here: bool,
) -> Result<VerificationStage> {
    // Cancellation of the parent request overrides a stale child, notably timeout and m.accepted.
    if let VerificationRequestState::Cancelled(info) = request {
        return Ok(cancelled(info, snapshot));
    }
    if let Some(sas) = &snapshot.sas {
        return match sas.state() {
            SasState::Created { .. } | SasState::Started { .. } | SasState::Accepted { .. } => {
                Ok(VerificationStage::Starting)
            }
            SasState::KeysExchanged { emojis, decimals } => Ok(VerificationStage::Compare {
                emoji: emojis
                    .map(|sas| sas.emojis.into_iter().map(emoji).collect())
                    .transpose()?
                    .unwrap_or_default(),
                decimals: vec![decimals.0, decimals.1, decimals.2],
            }),
            SasState::Confirmed => Ok(VerificationStage::WaitingForOther),
            SasState::Done { .. } => Ok(VerificationStage::FetchingKeys),
            SasState::Cancelled(info) => Ok(cancelled(&info, snapshot)),
        };
    }
    Ok(match request {
        VerificationRequestState::Created { .. } if started_here => VerificationStage::Requested,
        VerificationRequestState::Created { .. } | VerificationRequestState::Requested { .. } => {
            VerificationStage::Incoming
        }
        VerificationRequestState::Ready { .. } | VerificationRequestState::Transitioned { .. } => {
            VerificationStage::Starting
        }
        VerificationRequestState::Done => VerificationStage::FetchingKeys,
        VerificationRequestState::Cancelled(info) => cancelled(info, snapshot),
    })
}

fn emoji(emoji: Emoji) -> Result<VerificationEmoji> {
    let index = SAS_EMOJI
        .iter()
        .position(|symbol| *symbol == emoji.symbol)
        .ok_or_else(|| {
            Error::Upstream(format!("SDK 返回规范表以外的 SAS 表情：{}", emoji.symbol))
        })?;
    Ok(VerificationEmoji {
        index: index as u8,
        symbol: emoji.symbol.to_owned(),
    })
}

fn cancelled(info: &CancelInfo, snapshot: &Snapshot) -> VerificationStage {
    let reason = match info.cancel_code() {
        CancelCode::Accepted => VerificationCancel::AcceptedElsewhere,
        CancelCode::MismatchedSas => VerificationCancel::Mismatch,
        CancelCode::Timeout => VerificationCancel::Timeout,
        code if code.as_str() == super::concurrency::CANCEL_CODE => {
            VerificationCancel::ConcurrentRequest
        }
        CancelCode::User if snapshot.concurrent_remote => VerificationCancel::ConcurrentRequest,
        CancelCode::User if info.cancelled_by_us() && !snapshot.explicit_cancel => {
            VerificationCancel::ConcurrentRequest
        }
        CancelCode::User if info.cancelled_by_us() => VerificationCancel::ByMe,
        CancelCode::User => VerificationCancel::ByOther,
        _ => VerificationCancel::Other,
    };
    VerificationStage::Cancelled { reason }
}

pub(super) fn terminal(stage: &VerificationStage) -> bool {
    matches!(
        stage,
        VerificationStage::FetchingKeys | VerificationStage::Cancelled { .. }
    )
}
