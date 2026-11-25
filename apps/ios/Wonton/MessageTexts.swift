import Foundation
import WontonClient

extension SendFailure {
    /// 类别由核心给，说法在壳里；英文原文在 `SendState.Failed.details`，放进面板的标题。
    var text: LocalizedStringResource {
        switch self {
        case .offline: .Message1.messageSendFailureOffline
        case .tooLarge: .Message2.messageSendFailureTooLarge
        case .unsupportedType: .Message2.messageSendFailureUnsupportedType
        case .replyTarget: .Message1.messageSendFailureReplyTarget
        case .rejected: .Message0.messageSendFailureRejected
        case .upstream: .Message2.messageSendFailureUpstream
        case .other: .Message2.messageSendFailureOther
        }
    }
}
