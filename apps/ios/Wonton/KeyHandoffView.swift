import SwiftUI
import WontonClient

/// 加设备时等另一台设备交出聊天密钥（ADR-0087）：整屏盖住聊天列表，代替“输入恢复密钥”。该不该显示、走到哪一步
/// 都来自核心（`EncryptionStatus.keyHandoff` 和 `verificationFlow`）；主人选“稍后”只在这台 App 里先收起，
/// 聊天列表顶上的“在另一台设备上继续”再打开。
struct KeyHandoffCover: ViewModifier {
    let model: ChatModel
    /// “手边没有别的设备”：核心结束交接后去聊天备份页输恢复密钥。
    let openBackup: () -> Void

    func body(content: Content) -> some View {
        content.background {
            // 挂在单独的空视图上：和别的整屏页互不抢。
            Color.clear.fullScreenCover(isPresented: presented) {
                KeyHandoffView(model: model, openBackup: openBackup)
            }
        }
    }

    private var presented: Binding<Bool> {
        Binding(
            get: {
                guard case .receiving = model.viewModel.encryption.keyHandoff else { return false }
                return !model.keyHandoffDeferred
            },
            set: { _ in }
        )
    }
}

struct KeyHandoffView: View {
    let model: ChatModel
    let openBackup: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                Text(.Handoff.handoffTitle)
                    .wontonText(WontonTheme.typography.screenTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                progress
                Button {
                    model.send(.skipKeyHandoff)
                    openBackup()
                } label: {
                    Text(.Handoff.handoffNoOtherDevice)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                        .frame(maxWidth: .infinity, minHeight: WontonTheme.size.target)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("key-handoff-skip")
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(WontonTheme.spacing.pageInset)
        }
        .background(Color(colors.canvas).ignoresSafeArea())
    }

    @ViewBuilder private var progress: some View {
        let flow = model.viewModel.encryption.verificationFlow.flatMap { $0.startedHere ? $0 : nil }
        if case .receiving(unanswered: true) = model.viewModel.encryption.keyHandoff {
            WontonNotice(
                icon: .warning,
                title: String(localized: .Handoff.handoffUnansweredTitle),
                lines: [String(localized: .Handoff.handoffUnansweredMessage)]
            ) {
                HStack(spacing: WontonTheme.spacing.s8) {
                    Button { model.send(.startDeviceVerification) } label: {
                        Text(.Handoff.handoffTryAgain).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(WontonButtonStyle(kind: .primary))
                    Button { model.keyHandoffDeferred = true } label: {
                        Text(.Handoff.handoffLater).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            }
        } else if let flow, !waiting(flow.stage) {
            // 对方接上以后和聊天备份页里的确认一样：对表情、等对方、取密钥，没成时给“重新开始”。
            DeviceVerificationFlowView(flow: flow, model: model)
        } else {
            HStack(spacing: WontonTheme.spacing.s12) {
                ProgressView()
                Text(.Handoff.handoffWaiting)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
            }
        }
    }

    /// 请求发出还没人接：这时没有要主人在这台设备上做的，只说去另一台设备上。
    private func waiting(_ stage: VerificationStage) -> Bool {
        if case .requested = stage { return true }
        return false
    }
}
