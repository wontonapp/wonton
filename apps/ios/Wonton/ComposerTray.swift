import SwiftUI
import WontonClient

/// 输入框上方的附件区（ADR-0094）：一排小格，进来就开始传。每格右上角能拿掉，传不上去的点开重试。
struct ComposerTray: View {
    let composer: ComposerModel

    var body: some View {
        if !composer.attachments.isEmpty {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: WontonTheme.spacing.s8) {
                    ForEach(composer.attachments, id: \.id) { attachment in
                        AttachmentTile(
                            attachment: attachment, composer: composer,
                            remove: { composer.remove(attachment) }
                        )
                    }
                }
                .padding(.horizontal, WontonTheme.spacing.s12)
                .padding(.top, WontonTheme.spacing.s8)
            }
            .accessibilityIdentifier("composerTray")
        }
    }
}

/// 一个附件：照片画预览图，文件画图标和名字；上面盖着上传的进展。附件区和待发消息共用，`remove` 为空时不画拿掉的按钮。
struct AttachmentTile: View {
    let attachment: ComposerAttachment
    let composer: ComposerModel
    var remove: (() -> Void)?
    @State private var preview: UIImage?
    @State private var isShowingFailure = false
    @Environment(\.wontonColors) private var colors

    private let side = WontonTheme.size.previewIcon

    var body: some View {
        content
            .frame(width: side, height: side)
            .background(Color(colors.surfaceSubtle))
            .clipShape(RoundedRectangle(cornerRadius: WontonTheme.radii.control))
            .overlay { progress }
            .overlay(alignment: .topTrailing) {
                if let remove {
                    Button(action: remove) {
                        WontonIcon.close.view(size: WontonTheme.size.iconTiny)
                            .foregroundStyle(.white)
                            .padding(WontonTheme.spacing.s4)
                            .background(Circle().fill(.black.opacity(WontonTheme.ratio.scrim)))
                            .padding(WontonTheme.spacing.s4)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(Text(.Composer.composerAttachmentRemove(attachment.name)))
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel(Text(attachment.name))
            .task(id: attachment.id) {
                if case .image = attachment.kind { preview = await composer.thumbnail(attachment) }
            }
            .wontonSheet(isPresented: $isShowingFailure) { close in
                if case let .failed(reason, message) = attachment.state {
                    AttachmentFailureSheet(
                        reason: reason, message: message,
                        retry: { close { composer.retry(attachment) } },
                        remove: removeAfterClosing(close)
                    )
                }
            }
    }

    private func removeAfterClosing(_ close: @escaping (@escaping () -> Void) -> Void) -> (() -> Void)? {
        guard let remove else { return nil }
        return { close(remove) }
    }

    @ViewBuilder private var content: some View {
        switch attachment.kind {
        case .image:
            if let preview {
                Image(uiImage: preview).resizable().scaledToFill()
            } else {
                WontonIcon.image.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.textSecondary))
            }
        case .file:
            VStack(spacing: WontonTheme.spacing.s4) {
                WontonIcon.file.view(size: WontonTheme.size.iconSmall)
                Text(attachment.name)
                    .wontonText(WontonTheme.typography.caption)
                    .lineLimit(2)
                    .truncationMode(.middle)
                    .multilineTextAlignment(.center)
            }
            .foregroundStyle(Color(colors.textSecondary))
            .padding(WontonTheme.spacing.s4)
        }
    }

    /// 传的整个过程都画着：有比例画进度环，不知道总量时转圈；断了画暂停，失败画警告，点开说明和重试。
    @ViewBuilder private var progress: some View {
        switch attachment.state {
        case let .uploading(fraction):
            ZStack {
                Color.black.opacity(WontonTheme.ratio.scrim)
                if let fraction {
                    ProgressRing(fraction: Double(fraction))
                } else {
                    ProgressView().tint(.white)
                }
            }
            .allowsHitTesting(false)
        case .paused:
            ZStack {
                Color.black.opacity(WontonTheme.ratio.scrim)
                WontonIcon.pause.view(size: WontonTheme.size.iconSmall).foregroundStyle(.white)
            }
            .allowsHitTesting(false)
            .accessibilityElement()
            .accessibilityLabel(Text(.Composer.composerAttachmentPaused))
        case .ready:
            EmptyView()
        case .failed:
            Button {
                isShowingFailure = true
            } label: {
                ZStack {
                    Color.black.opacity(WontonTheme.ratio.scrim)
                    WontonIcon.warningCircleFill.view(size: WontonTheme.size.icon)
                        .foregroundStyle(Color(colors.danger))
                }
            }
            .buttonStyle(.plain)
            .accessibilityLabel(Text(.Composer.composerAttachmentFailed))
        }
    }
}

/// 上传进度环：底下一圈淡色，上面按比例描白。
private struct ProgressRing: View {
    let fraction: Double

    var body: some View {
        ZStack {
            Circle().stroke(.white.opacity(WontonTheme.ratio.scrim), lineWidth: 3)
            Circle()
                .trim(from: 0, to: min(max(fraction, 0), 1))
                .stroke(.white, style: StrokeStyle(lineWidth: 3, lineCap: .round))
                .rotationEffect(.degrees(-90))
        }
        .frame(width: WontonTheme.size.iconSmall + 8, height: WontonTheme.size.iconSmall + 8)
        .accessibilityElement()
        .accessibilityValue(Text(fraction, format: .percent.precision(.fractionLength(0))))
    }
}

/// 附件传不上去：一句人话说原因，详情收起，配重试和拿掉。
private struct AttachmentFailureSheet: View {
    let reason: AttachmentFailure
    let message: String
    let retry: () -> Void
    let remove: (() -> Void)?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            Text(reason.text)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
            ProblemDetail(text: message)
            HStack(spacing: WontonTheme.spacing.s8) {
                if let remove {
                    Button(action: remove) {
                        Text(.Common.commonDelete).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
                Button(action: retry) {
                    Text(.Composer.composerAttachmentRetry).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.bottom, WontonTheme.spacing.s8)
    }
}

extension AttachmentFailure {
    /// 类别由核心给，说法在壳里。
    var text: LocalizedStringResource {
        switch self {
        case .tooLarge: .Composer.composerAttachmentTooLarge
        case .sourceUnavailable: .Composer.composerAttachmentSourceUnavailable
        case .storageFull: .Composer.composerAttachmentStorageFull
        case .rejected: .Composer.composerAttachmentRejected
        case .upstream: .Composer.composerAttachmentUpstream
        case .localStorage: .Composer.composerAttachmentLocalStorage
        case .other: .Composer.composerAttachmentFailed
        }
    }
}
