import SwiftUI

/// 发送被拒这类操作失败的提示，挂在输入栏上方：字和附件留着，关掉继续（ADR-0109 第 10 节）。
/// 房间盖着旁聊时，标签页底下的 `ChatDialogs` 弹不出来，这里就地显示。
struct ComposerRejection: View {
    let problem: Problem
    let close: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
            ProblemRow(problem: problem)
            ChatGlassButton(icon: .close, label: String(localized: .Common.commonClose), action: close)
                .accessibilityIdentifier("composerRejectionClose")
        }
        .padding(WontonTheme.spacing.s12)
        .chatGlass(in: RoundedRectangle(cornerRadius: WontonTheme.radii.card, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("composerRejection")
    }
}
