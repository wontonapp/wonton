import SwiftUI
import WontonClient

/// 问 Bot 失败：按原因写一句人话，重试带同一个房间，关掉就收起。
struct AskFailureBar: View {
    @Bindable var model: ChatModel

    var body: some View {
        if let failure = model.askFailure {
            HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
                ProblemRow(
                    problem: Problem(
                        title: failure.reason.message,
                        detail: failure.detail.isEmpty ? nil : failure.detail
                    ),
                    retry: { try? model.retryAsk() }
                )
                .accessibilityIdentifier("askFailure")
                ChatGlassButton(
                    icon: .close,
                    label: String(localized: .Common.commonClose),
                    identifier: "askFailureClose"
                ) {
                    model.askFailure = nil
                }
            }
            .padding(WontonTheme.spacing.s12)
            .chatGlass(in: RoundedRectangle(cornerRadius: WontonTheme.radii.card, style: .continuous))
            .padding(.horizontal, WontonTheme.spacing.pageInset)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("askFailureBar")
        }
    }
}

private extension AskFailure {
    var message: String {
        switch self {
        case .noBot: String(localized: .Chat1.chatAskFailedNoBot)
        case .botNotReady: String(localized: .Chat1.chatAskFailedBotNotReady)
        case .botProblem: String(localized: .Chat1.chatAskFailedBotProblem)
        case .notInRoom: String(localized: .Chat1.chatAskFailedNotInRoom)
        case .readOnlyRoom: String(localized: .Chat1.chatAskFailedReadOnlyRoom)
        case .contactRequired: String(localized: .Chat1.chatAskFailedContactRequired)
        case .transportUnavailable: String(localized: .Chat1.chatAskFailedTransport)
        case .forbidden: String(localized: .Chat1.chatAskFailedForbidden)
        case .notReady: String(localized: .Chat1.chatAskFailedNotReady)
        case .upstream: String(localized: .Chat1.chatAskFailedUpstream)
        case .other: String(localized: .Chat1.chatAskFailedOther)
        }
    }
}
