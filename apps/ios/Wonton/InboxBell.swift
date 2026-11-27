import SwiftUI
import WontonClient

/// 主界面根页顶栏的收件箱铃铛（ADR-0119 第 9 节）：有等你回答的才在角上画数字；没有就是一枚普通的铃铛，
/// 不画 0。还没取到或取不成时数字不确定，也不画，进去看页面上的问题。
struct InboxBell: View {
    let inbox: InboxModel
    let open: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        WontonIconButton(icon: .inbox, label: label, action: open)
            .overlay(alignment: .topTrailing) {
                if let count = inbox.needing, count > 0 {
                    Text("\(count)")
                        .font(.system(size: 11, weight: .semibold).monospacedDigit())
                        .foregroundStyle(.white)
                        .padding(.horizontal, 5)
                        .frame(minWidth: 18, minHeight: 18)
                        .background(Color(colors.danger), in: Capsule())
                        // 顶栏的玻璃底会裁掉按钮框外的部分，角标收在框里，压住铃铛右上角。
                        .offset(x: -2, y: 4)
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
            .accessibilityIdentifier("inbox-bell")
            .accessibilityValue(inbox.needing.map { $0 > 0 ? String($0) : "" } ?? "")
    }

    private var label: String { String(localized: .Confirmations.confirmationsInboxTitle) }
}
