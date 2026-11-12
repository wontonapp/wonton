import SwiftUI
import WontonClient

struct TabFullscreenBar: View {
    let page: TabPage
    let tab: TabRef
    let tabs: TabsModel
    let details: () -> Void
    @Environment(\.wontonColors) private var colors
    @Environment(\.wontonBarStatus) private var status

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s8) {
            HStack(spacing: WontonTheme.spacing.s8) {
                if let title = status.title {
                    Text(title)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textPrimary))
                        .padding(.horizontal, WontonTheme.spacing.s12)
                        .padding(.vertical, WontonTheme.spacing.s8)
                        .background(.regularMaterial, in: Capsule())
                }
                Spacer(minLength: 0)
                capsule
            }
            if let line = status.line {
                line.padding(WontonTheme.spacing.s12)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: WontonTheme.radii.control))
            }
        }
    }

    private var capsule: some View {
        HStack(spacing: 0) {
            Menu {
                Button(TabText.reload) { tabs.send(.reloadTab(tab: tab)) }
                Toggle(TabText.alwaysFullscreen, isOn: Binding(
                    get: { page.opensFullscreen },
                    set: { tabs.send(.setTabDisplay(tab: tab, display: $0 ? .fullscreen : .standard)) }
                ))
                Button(TabText.details, action: details)
            } label: {
                WontonIcon.more.view()
                    .foregroundStyle(Color(colors.textPrimary))
                    .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
                    .contentShape(Rectangle())
            }
            .accessibilityLabel(TabText.more)
            Rectangle()
                .fill(Color(colors.separator))
                .frame(width: WontonTheme.size.hairline, height: WontonTheme.size.iconSmall)
            WontonIconButton(icon: .close, label: TabText.close) { tabs.send(.exitFullscreen(tab: tab)) }
        }
        // 看得见的胶囊比按钮矮，好放进灵动岛旁边；按钮的点按范围照旧，伸出胶囊上下。
        .frame(height: WontonTheme.size.smallButton)
        .background(.regularMaterial, in: Capsule())
        .overlay { Capsule().strokeBorder(Color(colors.separator), lineWidth: WontonTheme.size.hairline) }
    }
}
