import SwiftUI
import WontonClient

/// 点了发送、还没交给 Matrix 的一条（ADR-0094），画在时间线最后一条后面。附件各自画着上传的进展；交出去以后由 SDK 的本地回显代替。
/// 传不上去时整条变红，点开重发或者删掉。
struct OutgoingRow: View {
    let message: OutgoingMessage
    let composer: ComposerModel
    @State private var isChoosingFailed = false
    @State private var isChoosingSending = false
    @Environment(\.wontonColors) private var colors

    private var shape: some InsettableShape {
        RoundedRectangle(cornerRadius: WontonTheme.radii.bubble, style: .continuous)
    }

    var body: some View {
        HStack(spacing: 0) {
            Spacer(minLength: WontonTheme.spacing.s48)
            VStack(alignment: .trailing, spacing: WontonTheme.spacing.s4) {
                if !message.attachments.isEmpty {
                    LazyVGrid(
                        columns: Array(
                            repeating: GridItem(.fixed(WontonTheme.size.previewIcon), spacing: WontonTheme.spacing.s4),
                            count: min(message.attachments.count, 3)
                        ),
                        alignment: .trailing,
                        spacing: WontonTheme.spacing.s4
                    ) {
                        ForEach(message.attachments, id: \.id) { attachment in
                            AttachmentTile(attachment: attachment, composer: composer)
                        }
                    }
                }
                if !message.body.isEmpty {
                    Text(message.body)
                        .wontonText(WontonTheme.typography.body)
                        .foregroundStyle(Color(colors.textOnAccentFill))
                        .padding(.horizontal, WontonTheme.spacing.s12)
                        .padding(.vertical, WontonTheme.spacing.s8)
                        .accentFill(shape)
                }
                footer
            }
        }
        .accessibilityIdentifier("outgoingMessage")
    }

    private var footer: some View {
        HStack(spacing: WontonTheme.spacing.s4) {
            switch message.state {
            case let .sending(progress):
                // 传着的时候也能取消：整条不发，上传停下（盲测：取消了照样发出去）。
                Button {
                    isChoosingSending = true
                } label: {
                    if let progress {
                        Text(.Message2.messageSendingProgress(
                            Double(progress).formatted(.percent.precision(.fractionLength(0)))
                        ))
                    } else {
                        Text(.Message2.messageSending)
                    }
                }
                .buttonStyle(.plain)
                .wontonActionSheet(isPresented: $isChoosingSending, actions: [
                    WontonSheetAction(
                        title: String(localized: .Message2.messageCancelSend), icon: .close, isDanger: true
                    ) {
                        composer.cancel(message)
                    }
                ])
            case let .failed(reason, _):
                Button {
                    isChoosingFailed = true
                } label: {
                    HStack(spacing: WontonTheme.spacing.s4) {
                        WontonIcon.warningCircle.view(size: WontonTheme.size.iconTiny)
                        Text(.Message1.messageSendFailed(reason))
                    }
                    .foregroundStyle(Color(colors.danger))
                }
                .buttonStyle(.plain)
                .wontonActionSheet(isPresented: $isChoosingFailed, actions: [
                    WontonSheetAction(title: String(localized: .Message1.messageResend), icon: .refresh) {
                        composer.retry(message)
                    },
                    WontonSheetAction(title: String(localized: .Common.commonDelete), icon: .trash, isDanger: true) {
                        composer.cancel(message)
                    }
                ])
            case .sent:
                EmptyView()
            }
        }
        .wontonText(WontonTheme.typography.caption)
        .foregroundStyle(Color(colors.textSecondary))
    }
}
