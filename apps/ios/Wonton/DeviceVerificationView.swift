import SwiftUI
import WontonClient

/// SAS 表情的本地化名字：按 Matrix 规范 SAS 表情表的序号从字符串目录取（ADR-0046），名字不在代码里写。
private enum VerificationEmojiNames {
    static func name(_ index: Int) -> String {
        NSLocalizedString(
            "verification.emoji.\(index)",
            comment: "Name of the SAS emoji at this index in the Matrix spec table"
        )
    }
}

/// 一次设备确认进展到哪一步（ADR-0046）。发起的那台设备在聊天备份页显示；来确认的设备在“添加设备”页就地显示，
/// 或由 [DeviceVerificationDialogs] 弹出。
struct DeviceVerificationFlowView: View {
    let flow: VerificationFlow
    let model: ChatModel
    /// 确认取消后改用恢复密钥。只有发起确认的这台设备给：来确认的设备上没有下一步，结果给人看就够了。
    var useRecoveryKey: (() -> Void)?

    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            switch flow.stage {
            case .incoming: incomingRequest
            case .requested: waiting(Text(.verificationRequested), hint: Text(.verificationRequestedHint))
            case .starting: waiting(Text(.verificationStarting))
            case let .compare(emoji, decimals): compare(emoji, decimals)
            case .waitingForOther: waiting(Text(.verificationWaitingForOther))
            case .fetchingKeys: waiting(Text(.verificationFetchingKeys))
            case .done:
                (flow.startedHere ? Text(.verificationDoneHere) : Text(.verificationDoneOther))
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
            case let .cancelled(reason): cancelled(reason)
            case .other:
                Text(.verificationCancelledOther)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var incomingRequest: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            Text(incomingMessage)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            HStack(spacing: WontonTheme.spacing.s8) {
                Button { model.send(.acceptDeviceVerification(flowId: flow.flowId)) } label: {
                    Text(.verificationAccept).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
                Button { model.send(.cancelDeviceVerification(flowId: flow.flowId)) } label: {
                    Text(.verificationDeny).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
        }
    }

    /// 对方设备用配对时起的名字，没起过就用设备 ID。
    private var incomingMessage: String {
        let name = flow.otherDeviceName.flatMap { $0.isEmpty ? nil : $0 } ?? flow.otherDeviceId
        return String(localized: .verificationIncomingMessage(name))
    }

    private func waiting(_ text: Text, hint: Text? = nil) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            HStack(spacing: WontonTheme.spacing.s12) {
                ProgressView()
                text
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
            }
            if let hint {
                hint
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            cancelButton
        }
    }

    private func compare(_ emoji: [VerificationEmoji], _ decimals: [UInt16]) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            Text(.verificationCompareTitle)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            if emoji.isEmpty {
                // 对端不支持 SAS 表情时，规范回退成三个四位数。
                Text(decimals.map { String(format: "%04d", Int($0)) }.joined(separator: "  "))
                    .font(.body.monospaced())
                    .foregroundStyle(Color(colors.textPrimary))
            } else {
                WontonGroup {
                    ForEach(Array(emoji.enumerated()), id: \.offset) { index, item in
                        if index > 0 { WontonSeparator() }
                        HStack(spacing: WontonTheme.spacing.s12) {
                            Text(item.symbol)
                                .wontonText(WontonTheme.typography.itemTitle)
                                .foregroundStyle(Color(colors.textPrimary))
                            Text(VerificationEmojiNames.name(Int(item.index)))
                                .wontonText(WontonTheme.typography.body)
                                .foregroundStyle(Color(colors.textPrimary))
                        }
                        .padding(.horizontal, WontonTheme.spacing.s16)
                        .padding(.vertical, WontonTheme.spacing.s12)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }
            HStack(spacing: WontonTheme.spacing.s8) {
                Button { model.send(.confirmVerification(flowId: flow.flowId)) } label: {
                    Text(.verificationSame).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
                Button { model.send(.denyVerification(flowId: flow.flowId)) } label: {
                    Text(.verificationDifferent).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
            cancelButton
        }
    }

    private func cancelled(_ reason: VerificationCancel) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            reasonText(reason)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
            // 下一步只给发起确认的这台设备：“重新开始”是主按钮，恢复密钥是备选。没取全钥匙也先重来：可能只是请求丢了。
            if flow.startedHere {
                switch reason {
                case .acceptedElsewhere, .other:
                    EmptyView()
                default:
                    Button { model.send(.startDeviceVerification) } label: {
                        Text(.verificationRetry).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(WontonButtonStyle(kind: .primary))
                    if let useRecoveryKey {
                        Button(action: useRecoveryKey) {
                            Text(.verificationUseRecoveryKey).frame(maxWidth: .infinity)
                        }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                    }
                }
            }
        }
    }

    private var cancelButton: some View {
        Button { model.send(.cancelDeviceVerification(flowId: flow.flowId)) } label: {
            Text(.commonCancel).frame(maxWidth: .infinity)
        }
        .buttonStyle(WontonButtonStyle(kind: .secondary))
    }

    private func reasonText(_ reason: VerificationCancel) -> Text {
        switch reason {
        case .byMe: Text(.verificationCancelledByMe)
        case .byOther: Text(.verificationCancelledByOther)
        case .mismatch: Text(.verificationCancelledMismatch)
        case .timeout: Text(.verificationCancelledTimeout)
        case .acceptedElsewhere: Text(.verificationCancelledAcceptedElsewhere)
        case .concurrentRequest: Text(.verificationCancelledConcurrentRequest)
        case .keysMissing: Text(.verificationCancelledKeysMissing)
        case .failed: Text(.verificationCancelledFailed)
        case .other: Text(.verificationCancelledOther)
        }
    }
}

/// 来确认的设备收到的确认（ADR-0046）：请求在前台弹窗问“开始确认/拒绝”，接受后整屏显示对照和结果。
/// “添加设备”或聊天备份页开着时它们自己显示（[InviteView]、[ChatBackupView]），传 `suppressed: true` 不弹。
struct DeviceVerificationDialogs: ViewModifier {
    let model: ChatModel
    var suppressed = false
    @State private var dismissed: Set<String> = []

    func body(content: Content) -> some View {
        content
            .wontonDialog(
                String(localized: .verificationTitle),
                message: incomingPrompt,
                isPresented: incomingPresented,
                actions: [
                    WontonDialogAction(title: String(localized: .verificationAccept), kind: .primary) {
                        if let flow = currentFlow { model.send(.acceptDeviceVerification(flowId: flow.flowId)) }
                    },
                    WontonDialogAction(title: String(localized: .verificationDeny)) {
                        if let flow = currentFlow { model.send(.cancelDeviceVerification(flowId: flow.flowId)) }
                    }
                ]
            )
            .background {
                // 挂在单独的空视图上：和别的弹出页互不抢。
                Color.clear.fullScreenCover(item: presentation) { _ in
                    DeviceVerificationScreen(model: model)
                }
            }
    }

    private var currentFlow: VerificationFlow? {
        guard let flow = model.viewModel.encryption.verificationFlow, !flow.startedHere else { return nil }
        return flow
    }

    /// 对方设备用配对时起的名字，没起过就用设备 ID。
    private var incomingPrompt: String {
        guard let flow = currentFlow else { return "" }
        let name = flow.otherDeviceName.flatMap { $0.isEmpty ? nil : $0 } ?? flow.otherDeviceId
        return String(localized: .verificationIncomingMessage(name))
    }

    /// 请求必须由主人点按钮回答，点空白处不算回答，弹窗不走。
    private var incomingPresented: Binding<Bool> {
        Binding(
            get: {
                guard let flow = currentFlow, !suppressed, case .incoming = flow.stage else { return false }
                return true
            },
            set: { _ in }
        )
    }

    private var presentation: Binding<VerificationPresentation?> {
        Binding(
            get: {
                guard let flow = currentFlow, !suppressed, !dismissed.contains(flow.flowId) else { return nil }
                if case .incoming = flow.stage { return nil }
                return VerificationPresentation(id: flow.flowId)
            },
            set: {
                if $0 == nil, let flow = currentFlow { dismissed.insert(flow.flowId) }
            }
        )
    }

    private struct VerificationPresentation: Identifiable {
        let id: String
    }
}

/// 旧设备接受请求后的整屏进展，和聊天备份页里的一样。关掉不取消确认，进展回到聊天备份页还能看。
private struct DeviceVerificationScreen: View {
    let model: ChatModel

    var body: some View {
        ScrollView {
            if let flow = model.viewModel.encryption.verificationFlow {
                DeviceVerificationFlowView(flow: flow, model: model)
                    .padding(WontonTheme.spacing.pageInset)
            }
        }
        .wontonModalScreen(String(localized: .verificationTitle))
    }
}
