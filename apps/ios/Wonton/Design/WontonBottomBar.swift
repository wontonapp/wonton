import SwiftUI
import WontonClient

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

/// 主界面的底栏（ADR-0029）：浮起的系统材质，选中的一项是重点色的填充和实心图标。
struct WontonBottomBar<Tab: Hashable>: View {
    struct Item {
        let tab: Tab
        let label: String
        let icon: AnyView
        let selectedIcon: AnyView
        var badge = false
        var isDimmed = false
        var opensFullscreen: Bool?
    }

    let items: [Item]
    let labels: Bool
    @Binding var selection: Tab
    let reselect: (Tab) -> Void
    let longPress: (Tab) -> Void
    let enterFullscreen: (Tab) -> Void
    let setAlwaysFullscreen: (Tab, Bool) -> Void
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
            shape.fill(.regularMaterial)
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
            if isSelected { reselect(item.tab) } else { selection = item.tab }
        } label: {
            VStack(spacing: 2) {
                ZStack(alignment: .topTrailing) {
                    isSelected ? item.selectedIcon : item.icon
                    if item.badge {
                        Circle()
                            .fill(Color(colors.danger))
                            .frame(width: 7, height: 7)
                            .offset(x: 5, y: -2)
                    }
                }
                if labels {
                    Text(item.label).wontonText(WontonTheme.typography.caption).lineLimit(1)
                }
            }
            .foregroundStyle(color)
            .opacity(item.isDimmed ? 0.45 : 1)
            .frame(maxWidth: .infinity, minHeight: WontonTheme.size.target)
            .padding(.vertical, labels ? WontonTheme.spacing.s4 : 0)
            .background {
                if isSelected {
                    Color.clear.accentFill(RoundedRectangle(cornerRadius: WontonTheme.radii.barItem), border: false)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .contextMenu {
            if let opensFullscreen = item.opensFullscreen {
                Button(TabText.fullscreen) { enterFullscreen(item.tab) }
                Toggle(TabText.alwaysFullscreen, isOn: Binding(
                    get: { opensFullscreen }, set: { setAlwaysFullscreen(item.tab, $0) }
                ))
            }
            Button(TabText.bottomBar) { longPress(item.tab) }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(item.label)
        .accessibilityAddTraits(isSelected ? [.isButton, .isSelected] : .isButton)
        .accessibilityShowsLargeContentViewer {
            isSelected ? item.selectedIcon : item.icon
            Text(item.label)
        }
    }
}
