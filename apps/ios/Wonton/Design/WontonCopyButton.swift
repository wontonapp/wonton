import SwiftUI
import UniformTypeIdentifiers

/// App 里所有的“复制”按钮：点了先写进剪贴板，再把图标换成对勾、文字换成“已复制”，带一下轻触觉，过一会儿复原。
/// 不给反馈时主人点了以为没反应（11-27 反馈恢复密钥的复制）。Android 是同名的 `WontonCopyButton`，反馈一致。
struct WontonCopyButton: View {
    enum Clipboard {
        /// 要贴到别的设备上的（配对文本、消息里的代码）：照常走通用剪贴板。
        case shared
        /// 恢复密钥、插件生成的密钥：只留在本机剪贴板（不经通用剪贴板同步到别的设备），两分钟后系统清掉。
        case localSecret
    }

    enum Look {
        /// 和旁边的主按钮并排、撑满一半宽的次按钮。
        case wide
        /// 小胶囊按钮。
        case small
        /// 只有图标（代码块右上角）；`tint` 跟着所在的底色。
        case icon(tint: Color?)
    }

    let text: String
    var clipboard: Clipboard = .shared
    var look: Look = .small
    /// 没复制时读屏念的名字；不给就是“复制”。
    var label: String?
    /// 每点一次加一：再点一次重新计时，触觉也再响一次。
    @State private var copies = 0
    @State private var copied = false

    var body: some View {
        styled
            .sensoryFeedback(.success, trigger: copies) { _, count in count > 0 }
            .task(id: copies) {
                guard copies > 0 else { return }
                copied = true
                // 1.5 秒：够看清对勾，又不会让人以为还要再点什么（两端同一个时长）。
                try? await Task.sleep(for: .milliseconds(1500))
                guard !Task.isCancelled else { return }
                copied = false
            }
    }

    @ViewBuilder private var styled: some View {
        switch look {
        case .wide:
            Button(action: copy) { title.frame(maxWidth: .infinity) }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
        case .small:
            Button(action: copy) { title }
                .buttonStyle(WontonSmallButtonStyle())
        case let .icon(tint):
            WontonIconButton(icon: copied ? .check : .copy, label: currentLabel, tint: tint, action: copy)
        }
    }

    /// 图标和文字一起换；两种文字叠着占位，换的时候按钮不变宽、不挤旁边的按钮。
    private var title: some View {
        ZStack {
            labelRow(copied: false).hidden()
            labelRow(copied: true).hidden()
            labelRow(copied: copied)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(currentLabel)
    }

    private func labelRow(copied: Bool) -> some View {
        HStack(spacing: WontonTheme.spacing.s4) {
            (copied ? WontonIcon.check : .copy).view(size: WontonTheme.size.iconSmall)
            Text(copied ? String(localized: .Design.designCopied) : String(localized: .Design.designCopy))
        }
    }

    private var currentLabel: String {
        copied ? String(localized: .Design.designCopied) : (label ?? String(localized: .Design.designCopy))
    }

    private func copy() {
        switch clipboard {
        case .shared:
            UIPasteboard.general.string = text
        case .localSecret:
            // 和 `UIPasteboard.string` 写的是同一种类型：别的 App 按纯文本粘贴时认得出。
            UIPasteboard.general.setItems(
                [[UTType.utf8PlainText.identifier: text]],
                options: [.localOnly: true, .expirationDate: Date.now.addingTimeInterval(120)]
            )
        }
        copies += 1
    }
}
