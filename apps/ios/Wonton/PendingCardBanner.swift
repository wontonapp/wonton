import SwiftUI
import WontonClient

/// 有卡片等主人回答时顶在页面最上面，点了去卡片所在的房间。多张时去最早到期的那张。
struct PendingCardBanner: View {
    let cards: [PendingConfirmation]
    let open: (String) -> Void

    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button {
            open(cards[0].roomId)
        } label: {
            HStack(spacing: 8) {
                WontonIcon.notify.view(size: 20)
                Text(title)
                    .font(.system(size: 15))
                    .lineLimit(2)
                    .frame(maxWidth: .infinity, minHeight: 24, alignment: .leading)
                WontonIcon.forward.view(size: 16)
            }
            .foregroundStyle(Color(colors.textOnAccentFill))
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .background(alignment: .bottom) {
            if let border = colors.accentFillBorder {
                Color(border).frame(height: 1)
            }
        }
        .background(Color(colors.accentFill), ignoresSafeAreaEdges: .top)
    }

    /// 只有一张卡片时说是什么事。
    private var title: String {
        cards.count == 1 ? "\(cards[0].title)，等你确认" : "\(cards.count) 张卡片等你确认"
    }
}
