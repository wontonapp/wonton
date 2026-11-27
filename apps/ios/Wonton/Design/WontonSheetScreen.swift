import SwiftUI

extension View {
    /// 系统 sheet 里 NavigationStack 的根页面（设置）：标题居中，关闭在右上角，左上角留给子页的返回。
    /// 下拉关闭的跟手、阻尼、松手按位置和速度判定，以及和列表滚动的先后，都交给系统 sheet，不自己算。
    @ViewBuilder func wontonSheetScreen(_ title: String) -> some View {
        if #available(iOS 26, *) {
            modifier(WontonNativeSheetBar(title: title))
                .modifier(WontonCanvas())
        } else {
            wontonTopBar { WontonSheetBar(title: title) }
                .toolbar(.hidden, for: .navigationBar)
                .modifier(WontonCanvas())
        }
    }
}

/// iOS 26：系统栏，关闭是栏里的标准按钮，材质和按压反馈由系统给（ADR-0107）。
@available(iOS 26, *)
private struct WontonNativeSheetBar: ViewModifier {
    let title: String
    @Environment(\.wontonColors) private var colors
    @Environment(\.dismiss) private var dismiss

    func body(content: Content) -> some View {
        content
            .navigationTitle("")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar(.visible, for: .navigationBar)
            .toolbar {
                ToolbarItem(placement: .principal) {
                    Text(title)
                        .wontonText(WontonTheme.typography.itemTitle)
                        .foregroundStyle(Color(colors.textPrimary))
                        .lineLimit(1)
                        .accessibilityAddTraits(.isHeader)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    WontonIconButton(icon: .close, label: String(localized: .Design.designClose)) { dismiss() }
                        .environment(\.wontonSystemToolbar, true)
                }
            }
    }
}

/// iOS 17–18：自绘栏，和别的自绘栏一样垫系统材质（`wontonTopBar`）。
private struct WontonSheetBar: View {
    let title: String
    @Environment(\.wontonColors) private var colors
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        ZStack {
            Text(title)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
                .lineLimit(1)
                .accessibilityAddTraits(.isHeader)
                .padding(.horizontal, WontonTheme.size.target + WontonTheme.spacing.s8)
            HStack(spacing: 0) {
                Spacer(minLength: 0)
                WontonIconButton(icon: .close, label: String(localized: .Design.designClose)) { dismiss() }
            }
            .padding(.horizontal, WontonTheme.spacing.s4)
        }
        .frame(minHeight: WontonTheme.size.topBar)
    }
}
