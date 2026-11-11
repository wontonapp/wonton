import SwiftUI
import WontonClient

extension EnvironmentValues {
    /// 根页面左上角打开侧边栏的按钮；没有时不显示。
    @Entry var wontonOpenSidebar: (() -> Void)?
    /// 侧边栏的收起拖动刚被认出来：这次触点抬手时不再算点按（左滑收起不误进应用）。
    @Entry var wontonSidebarTapCancelled: Bool = false
}

/// 侧边栏打开时把主界面整页推到右边、缩小、变圆角，只露出一条（学 ChatGPT、Grok）。
struct SidebarContainer<Main: View, Panel: View>: View {
    @Binding var isOpen: Bool
    /// 根页面才响应左边缘右滑；进了下一层，左边缘留给系统的返回手势。
    let edgeSwipe: Bool
    @ViewBuilder let main: Main
    @ViewBuilder let panel: Panel
    @State private var drag: CGFloat = 0
    @State private var isDragging = false
    @State private var tapCancelled = false
    @Environment(\.wontonColors) private var colors
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private let widthRatio: CGFloat = 0.82
    private let edgeWidth: CGFloat = 20

    var body: some View {
        GeometryReader { geometry in
            let width = geometry.size.width * widthRatio
            let progress = progress(width: width)
            ZStack(alignment: .leading) {
                Color(colors.surfaceSubtle).ignoresSafeArea()
                panel
                    .environment(\.wontonSidebarTapCancelled, tapCancelled)
                    .frame(width: width)
                    .offset(x: -width * 0.18 * (1 - progress))
                    .opacity(0.3 + 0.7 * progress)
                    .allowsHitTesting(progress > 0.5)
                    .accessibilityHidden(!isOpen)
                main
                    .allowsHitTesting(progress == 0)
                    .overlay { scrim(progress) }
                    .clipShape(RoundedRectangle(cornerRadius: 40 * progress, style: .continuous))
                    .shadow(color: .black.opacity(0.18 * progress), radius: 24, x: -4)
                    .scaleEffect(1 - 0.06 * progress, anchor: .leading)
                    .offset(x: width * progress)
                if edgeSwipe, !isOpen {
                    Color.clear
                        .frame(width: edgeWidth)
                        .contentShape(Rectangle())
                        .gesture(dragGesture(width: width))
                }
            }
            .simultaneousGesture(dragGesture(width: width), including: isOpen ? .all : .subviews)
        }
        .sensoryFeedback(.impact(weight: .light), trigger: isOpen)
    }

    private func progress(width: CGFloat) -> CGFloat {
        let base = isOpen ? width : 0
        return min(max((base + drag) / width, 0), 1)
    }

    /// 露出来的那一条：盖一层淡暗色，点一下收起。
    @ViewBuilder private func scrim(_ progress: CGFloat) -> some View {
        if progress > 0 {
            Color.black.opacity(0.22 * progress)
                .contentShape(Rectangle())
                .onTapGesture { set(open: false) }
                .accessibilityLabel(String(localized: .sidebarCollapse))
                .accessibilityAddTraits(.isButton)
        }
    }

    /// 跟手：松开时按位置和速度决定打开还是弹回去。
    private func dragGesture(width: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 8)
            .onChanged { value in
                guard abs(value.translation.width) > abs(value.translation.height) || isDragging else { return }
                isDragging = true
                drag = value.translation.width
                // 横向拖动一旦被认作收起手势，这次触点抬手时不再算点按： simultaneousGesture 不拦按钮，
                // 抬手的点按由面板那侧看这个标记自己放弃。
                tapCancelled = true
            }
            .onEnded { value in
                guard isDragging else { return }
                isDragging = false
                let projected = (isOpen ? width : 0) + value.predictedEndTranslation.width
                set(open: projected > width / 2)
                // 抬手时按钮动作和拖动的 onEnded 先后不定：留一小段窗口让被取消的点按落进来；
                // 窗口过了点按照常。拖动还在走时标记会被 onChanged 立刻置回。
                Task { @MainActor in
                    try? await Task.sleep(for: .milliseconds(350))
                    tapCancelled = false
                }
            }
    }

    private func set(open: Bool) {
        withAnimation(reduceMotion ? .easeOut(duration: 0.15) : .spring(response: 0.38, dampingFraction: 0.86)) {
            isOpen = open
            drag = 0
        }
    }
}

/// 侧边栏里的内容：主要入口、我的应用，底下是设置和新聊天。列表和角标都是核心算好的（`PluginsViewModel.sidebar`）。
struct SidebarPanel: View {
    let home: HomeModel
    let plugins: PluginsModel
    let current: TabRef
    let navigate: (SidebarDestination) -> Void
    @Environment(TabsModel.self) private var tabs
    @Environment(\.wontonColors) private var colors
    @Environment(\.wontonSidebarTapCancelled) private var tapCancelled
    @State private var pin: TabRef?

    /// 收起拖动认出来之后的抬手不算点按：左滑收起侧边栏时不误进应用；轻点和竖滑照常。
    private func openDestination(_ destination: SidebarDestination) {
        guard !tapCancelled else { return }
        navigate(destination)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
                .padding(.horizontal, WontonTheme.spacing.s24)
                .padding(.top, WontonTheme.spacing.s16)
                .padding(.bottom, WontonTheme.spacing.s24)
            ScrollView {
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                    SidebarRow(title: TabText.chat, selected: current == .chat) {
                        (current == .chat ? WontonIcon.chatFill : WontonIcon.chat).view()
                    } action: { openDestination(.chat) }
                    SidebarRow(
                        title: String(localized: .storeTitle),
                        badge: Int(sidebar.updates),
                        selected: false
                    ) {
                        WontonIcon.store.view()
                    } action: { openDestination(.store) }
                    myApps
                }
                .padding(.horizontal, WontonTheme.spacing.s12)
            }
            .scrollIndicators(.hidden)
            footer
                .padding(.horizontal, WontonTheme.spacing.s16)
                .padding(.vertical, WontonTheme.spacing.s12)
        }
        .tabPinRequests($pin)
        .task {
            plugins.refresh(.refreshPlugins)
            plugins.refresh(.refreshStore(force: false))
        }
    }

    private var sidebar: SidebarViewModel { plugins.viewModel.sidebar }

    private var header: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            Text(.sidebarTitle)
                .wontonText(WontonTheme.typography.screenTitle)
                .foregroundStyle(Color(colors.textPrimary))
            HStack(spacing: 6) {
                Circle().fill(statusColor).frame(width: 7, height: 7)
                Text(statusText)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
        }
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder private var myApps: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(.sidebarMyApps)
                .wontonText(WontonTheme.typography.supporting)
                .fontWeight(.semibold)
                .foregroundStyle(Color(colors.textSecondary))
                .accessibilityAddTraits(.isHeader)
            Spacer(minLength: 0)
            Button(String(localized: .sidebarAll)) { openDestination(.allApps) }
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.accentInk))
                .buttonStyle(.plain)
        }
        .padding(.horizontal, WontonTheme.spacing.s12)
        .padding(.top, WontonTheme.spacing.s24)
        .padding(.bottom, WontonTheme.spacing.s4)
        ForEach(sidebar.apps, id: \.pluginId) { app in
            SidebarRow(
                title: app.title,
                badge: app.update ? 1 : 0,
                dotOnly: true,
                selected: selected(app),
                pinned: isPinned(app)
            ) {
                PluginIconView(icon: app.icon, size: 28, model: plugins)
            } action: {
                openDestination(.app(app))
            }
            .contextMenu { pinMenu(app) }
        }
        if sidebar.moreApps {
            SidebarRow(title: String(localized: .sidebarAllApps), selected: current == .apps) {
                WontonIcon.apps.view()
            } action: { openDestination(.allApps) }
        }
    }

    private func selected(_ app: SidebarApp) -> Bool {
        guard case let .page(tab) = app.open else { return false }
        return tab == current
    }

    private func isPinned(_ app: SidebarApp) -> Bool {
        guard case let .page(tab) = app.open else { return false }
        return tabs.isPinned(tab)
    }

    @ViewBuilder private func pinMenu(_ app: SidebarApp) -> some View {
        let plugin = plugins.viewModel.installed?.plugins.first { $0.id == app.pluginId }
        ForEach(plugin?.tabChoices ?? [], id: \.tab) { choice in
            let pinned = tabs.isPinned(choice.tab)
            Button {
                if pinned { tabs.send(.unpinTab(tab: choice.tab)) } else { pin = choice.tab }
            } label: {
                Label(pinned ? TabText.unpin : TabText.pin, systemImage: pinned ? "pin.slash" : "pin")
            }
        }
    }

    private var footer: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            Button { openDestination(.settings) } label: {
                WontonIcon.settings.view()
                    .foregroundStyle(Color(colors.textPrimary))
                    .frame(width: WontonTheme.size.target, height: WontonTheme.size.target)
                    .background(Color(colors.surface), in: Circle())
                    .overlay {
                        Circle().strokeBorder(Color(colors.separator), lineWidth: WontonTheme.size.hairline)
                    }
                    .overlay(alignment: .topTrailing) {
                        if sidebar.settingsBadge {
                            Circle().fill(Color(colors.danger)).frame(width: 8, height: 8)
                                .accessibilityLabel(String(localized: .sidebarNeedsAttention))
                        }
                    }
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(TabText.settings)
            Spacer(minLength: 0)
            Button { openDestination(.newChat) } label: {
                HStack(spacing: WontonTheme.spacing.s8) {
                    WontonIcon.compose.view(size: WontonTheme.size.iconSmall)
                    Text(.chatNewChat).wontonText(WontonTheme.typography.itemTitle)
                }
                .foregroundStyle(Color(colors.onPrimary))
                .padding(.horizontal, WontonTheme.spacing.s24)
                .frame(height: WontonTheme.size.target)
                .background(Color(colors.primary), in: Capsule())
                .contentShape(Capsule())
            }
            .buttonStyle(.plain)
        }
    }

    private var statusText: String {
        guard home.viewModel.status != .notConfigured else {
            return String(localized: .chatNotSignedIn)
        }
        let server = String(localized: .serversDefaultFirst)
        return "\(server) · \(home.viewModel.status.connectionText)"
    }

    private var statusColor: Color {
        switch home.viewModel.status {
        case .connected: Color(colors.success)
        case .connecting: Color(colors.warning)
        case .notConfigured, .disconnected: Color(colors.danger)
        }
    }
}

enum SidebarDestination {
    case chat
    case store
    case allApps
    case settings
    case newChat
    case app(SidebarApp)
}

private struct SidebarRow<Icon: View>: View {
    let title: String
    var badge = 0
    var dotOnly = false
    let selected: Bool
    var pinned = false
    @ViewBuilder let icon: Icon
    let action: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button(action: action) {
            HStack(spacing: WontonTheme.spacing.s12) {
                icon.frame(width: 28, height: 28)
                Text(title)
                    .wontonText(WontonTheme.typography.body)
                    .fontWeight(selected ? .semibold : .regular)
                    .lineLimit(1)
                Spacer(minLength: 0)
                if pinned {
                    WontonIcon.pinFill.view(size: 11)
                        .foregroundStyle(Color(colors.textSecondary))
                        .accessibilityLabel(String(localized: .sidebarPinned))
                }
                if badge > 0 {
                    if dotOnly {
                        Circle().fill(Color(colors.danger)).frame(width: 8, height: 8)
                            .accessibilityLabel(String(localized: .sidebarUpdatable))
                    } else {
                        Text("\(badge)")
                            .font(.system(size: 13, weight: .semibold).monospacedDigit())
                            .foregroundStyle(.white)
                            .padding(.horizontal, 7)
                            .frame(minWidth: 22, minHeight: 22)
                            .background(Color(colors.danger), in: Capsule())
                            .accessibilityLabel(String(localized: .launcherUpdatesAvailable(count: badge)))
                    }
                }
            }
            .foregroundStyle(Color(colors.textPrimary))
            .padding(.horizontal, WontonTheme.spacing.s12)
            .frame(minHeight: 44)
            .background {
                if selected {
                    RoundedRectangle(cornerRadius: WontonTheme.radii.control, style: .continuous)
                        .fill(Color(colors.surface))
                }
            }
            .contentShape(RoundedRectangle(cornerRadius: WontonTheme.radii.control))
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}
