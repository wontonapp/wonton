import SwiftUI

/// 网页上面浮着的连接状态那一行：核心给了成形的一行就用它，否则是一句话。两样都没有时什么都不画。
struct PageStatusLine: View {
    @Environment(\.wontonColors) private var colors
    @Environment(\.wontonBarStatus) private var status

    var body: some View {
        if let line = status.line {
            line.padding(WontonTheme.spacing.s12)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: WontonTheme.radii.control))
        } else if let sentence = status.subtitle {
            Text(sentence)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(WontonTheme.spacing.s12)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: WontonTheme.radii.control))
        }
    }
}
