import QuickLook
import SwiftUI

/// 文件只在确认后交给上传器；查看预览和取消都不启动传输。
struct PasteFilesPreview: View {
    let urls: [URL]
    let onSend: () -> Void
    let onCancel: () -> Void
    @State private var preview: URL?

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s16) {
            ScrollView {
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
                    ForEach(Array(urls.enumerated()), id: \.offset) { _, url in
                        Button { preview = url } label: {
                            Label {
                                Text(url.lastPathComponent)
                                    .multilineTextAlignment(.leading)
                            } icon: {
                                WontonIcon.attach.view(size: WontonTheme.size.iconSmall)
                            }
                            .frame(maxWidth: .infinity, minHeight: WontonTheme.size.target, alignment: .leading)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            .frame(maxHeight: WontonTheme.size.screenshot)
            HStack(spacing: WontonTheme.spacing.s8) {
                Button(action: onCancel) {
                    Text(.commonCancel).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                Button(action: onSend) {
                    Text(.composerSend).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .primary))
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.bottom, WontonTheme.spacing.s8)
        .quickLookPreview($preview)
    }
}
