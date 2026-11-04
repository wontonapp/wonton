import SwiftUI

private struct WontonTabBarKey: EnvironmentKey {
    static var defaultValue: AnyView? { nil }
}

extension EnvironmentValues {
    /// 根页面要放的底栏。放在 `TabView` 外面的话，它的安全区传不到各页的滚动区，最后一行会被挡住（iOS 26.5 模拟器实测）；
    /// 放在根页面里，进入下一层时它也跟着走。
    var wontonTabBar: AnyView? {
        get { self[WontonTabBarKey.self] }
        set { self[WontonTabBarKey.self] = newValue }
    }
}

/// 主界面的底栏（ADR-0026）：浮起的表面加噪点，选中的一项是重点色的填充和实心图标。
struct WontonBottomBar<Tab: Hashable>: View {
    struct Item {
        let tab: Tab
        let label: String
        let icon: WontonIcon
        let selectedIcon: WontonIcon
    }

    let items: [Item]
    @Binding var selection: Tab
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: 0) {
            ForEach(items, id: \.tab) { item in
                button(item)
            }
        }
        .padding(WontonTheme.spacing.s8)
        .background {
            let shape = RoundedRectangle(cornerRadius: WontonTheme.radii.bar)
            shape.fill(Color(colors.surface))
                .overlay { NoiseTile().clipShape(shape) }
                .overlay { shape.strokeBorder(Color(colors.separator), lineWidth: WontonTheme.size.hairline) }
                .shadow(color: .black.opacity(0.08), radius: 14, y: 4)
        }
        .padding(.horizontal, WontonTheme.spacing.s16)
        .padding(.bottom, WontonTheme.spacing.s4)
    }

    private func button(_ item: Item) -> some View {
        let isSelected = item.tab == selection
        let color = Color(isSelected ? colors.accentInkOnFill : colors.textSecondary)
        return Button {
            selection = item.tab
        } label: {
            VStack(spacing: 2) {
                (isSelected ? item.selectedIcon : item.icon).view()
                Text(item.label).wontonText(WontonTheme.typography.caption)
            }
            .foregroundStyle(color)
            .frame(maxWidth: .infinity, minHeight: WontonTheme.size.target)
            .padding(.vertical, WontonTheme.spacing.s4)
            .background {
                if isSelected {
                    Color.clear.accentFill(RoundedRectangle(cornerRadius: WontonTheme.radii.barItem), border: false)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(item.label)
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
        .accessibilityShowsLargeContentViewer {
            (isSelected ? item.selectedIcon : item.icon).view()
            Text(item.label)
        }
    }
}
