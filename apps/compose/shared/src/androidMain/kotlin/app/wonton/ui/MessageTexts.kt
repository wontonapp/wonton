package app.wonton.ui

import app.wonton.protocol.SendFailure

/** 类别由核心给，说法在壳里；英文原文在 [_SendState_Failed.details]，放进面板的标题。 */
internal val SendFailure.text: Int
    get() = when (this) {
        SendFailure.OFFLINE -> R.string.timeline_send_failure_offline
        SendFailure.TOO_LARGE -> R.string.timeline_send_failure_too_large
        SendFailure.UNSUPPORTED_TYPE -> R.string.timeline_send_failure_unsupported_type
        SendFailure.REPLY_TARGET -> R.string.timeline_send_failure_reply_target
        SendFailure.REJECTED -> R.string.timeline_send_failure_rejected
        SendFailure.UPSTREAM -> R.string.timeline_send_failure_upstream
        SendFailure.OTHER -> R.string.timeline_send_failure_other
    }
