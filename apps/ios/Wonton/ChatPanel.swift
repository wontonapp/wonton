import SwiftUI
import UIKit
import WebKit
import WontonClient

/// 打开房间面板要的一整套模型和回调：房间在导航栈里，从环境取，不再一层层传。
struct PanelDependencies {
    let tabs: TabsModel
    let home: HomeModel
    let plugins: PluginsModel
    let chat: ChatModel
    let appearance: AppearanceModel
    let draft: (String) -> Void
    let open: (PluginEntry) -> Void
    let pendingCards: [PendingConfirmation]
    let openCardRoom: (String) -> Void
}

extension EnvironmentValues {
    @Entry var panelDeps: PanelDependencies?
}

struct TabPanelMode {
    let expanded: Bool
    /// 这一页是当前页、面板也拉着：加载好或被翻到时发“被看到了”，页面据此刷新（ADR-0052）。
    let visible: Bool
    /// 网页框顶边在面板里的位置：第 2 级让出状态栏，第 1 级贴着面板顶边。
    let insetTop: CGFloat
    let insetBottom: CGFloat
    /// 继续拖动网页时监听哪一端；第 1 级往打开方向、第 2 级往收回方向。
    let pagePull: Bool
    let pageEdge: PanelEdge
    let pageChanged: (CGFloat) -> Void
    let pageEnded: (CGFloat, CGFloat) -> Void
    let close: () -> Void
    let edit: () -> Void
}

enum PanelLevel { case closed, peek, full }

/// 聊天页的下拉面板（ADR-0052）：第 0 级收起，第 1 级露出一条聊天页，第 2 级整页。
/// 主聊天页从上方下来，房间从下方出来，两处上下对调。
struct ChatPanelHost<Content: View>: View {
    let tabs: TabsModel
    let home: HomeModel
    let plugins: PluginsModel
    let chat: ChatModel
    let appearance: AppearanceModel
    let scope: PanelScope
    let edge: PanelEdge
    /// 面板里的页面（主面板来自 `TabsViewModel.panel`，房间来自每一批时间线）。
    let items: [PanelItem]
    /// 核心记住的上次看的那一页。
    let selected: TabRef?
    /// 房间里盖着列表的东西开着：入口不响应（ADR-0052）。
    let blocked: Bool
    let draft: (String) -> Void
    let open: (PluginEntry) -> Void
    let pendingCards: [PendingConfirmation]
    let openCardRoom: (String) -> Void
    let select: (TabRef) -> Void
    let remove: (TabRef) -> Void
    let move: (TabRef, Int) -> Void
    /// 房间把消息列表的 `PullAnchor` 交进来：拖动手势要接到这个面板，不能按环境去找（房间里环境里
    /// 还是外面主聊天页的那个面板）。
    let pullAnchor: PullAnchor?
    @ViewBuilder let content: () -> Content
    @State private var level = PanelLevel.closed
    @State private var drag: CGFloat = 0
    @State private var mounted = false
    @State private var selection: TabRef?
    @State private var editing = false
    @State private var webViews = TabWebViewStore()
    @State private var safeTop: CGFloat = 0
    @State private var screenHeight: CGFloat = 0
    @State private var armed = PanelLevel.closed
    @State private var buzzes = 0
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.wontonColors) private var colors
    private static var mainPeekChatFraction: CGFloat { 0.18 }
    /// 房间第 1 级让面板占约 55%，聊天仍留在上方约 45%。
    private static var roomPeekChatFraction: CGFloat { 0.45 }
    private static var pullThreshold: CGFloat { 80 }
    private static var pushThreshold: CGFloat { 50 }

    var body: some View {
        GeometryReader { geometry in
            // 聊天页在安全区里排版，移动的距离按整屏算。
            let height = geometry.size.height + geometry.safeAreaInsets.top + geometry.safeAreaInsets.bottom
            ZStack(alignment: .top) {
                Color(colors.canvas).ignoresSafeArea()
                if mounted {
                    pager(height: height, safe: geometry.safeAreaInsets)
                        .offset(y: edge == .bottom ? max(0, height - chatOffset(height)) : 0)
                        .ignoresSafeArea()
                }
                chat(height: height)
                if level != .closed {
                    strip(height: height)
                }
                attention(height: height)
                #if DEBUG
                    BuzzMark(count: buzzes)
                #endif
            }
            .environment(\.chatPull, pull(height: height))
        }
        .onGeometryChange(for: CGFloat.self) { $0.safeAreaInsets.top } action: { safeTop = $0 }
        .onGeometryChange(for: CGFloat.self) {
            $0.size.height + $0.safeAreaInsets.top + $0.safeAreaInsets.bottom
        } action: { screenHeight = $0 }
        .sheet(isPresented: $editing) { editor }
        .onAppear { syncSelection() }
        .onChange(of: items.map(\.tab)) {
            syncSelection()
            retainWindow()
        }
        .onChange(of: selected) { syncSelection() }
        .onChange(of: items.isEmpty) { _, empty in if empty, level == .full { move(to: .full) } }
        .onChange(of: selection) { _, value in
            retainWindow()
            if level != .closed, let value { select(value) }
        }
        .onChange(of: level) { _, value in
            if value != .closed, let selection { select(selection) }
        }
        .onDisappear { webViews.releaseAll() }
        .preference(key: ChatPanelOpenKey.self, value: level != .closed || drag > 0)
    }

    private var sign: CGFloat { edge == .top ? 1 : -1 }

    private func pull(height: CGFloat) -> ChatPull {
        ChatPull(
            enabled: level == .closed && !blocked,
            changed: { distance in pullChanged(distance) },
            ended: { distance, velocity in settle(distance: distance, velocity: velocity, height: height) }
        )
    }

    // MARK: - 级

    private func base(_ level: PanelLevel, _ height: CGFloat) -> CGFloat {
        switch level {
        case .closed: return 0
        case .peek:
            let chatFraction = edge == .top ? Self.mainPeekChatFraction : Self.roomPeekChatFraction
            return height * (1 - chatFraction) - (edge == .top ? safeTop : 0)
        case .full: return height
        }
    }

    private func chatOffset(_ height: CGFloat) -> CGFloat {
        // 空面板最多到第 1 级，聊天页也不跟着拉过那一条。
        let limit = items.isEmpty ? base(.peek, height) : height
        return min(limit, max(0, base(level, height) + drag))
    }

    private func pullChanged(_ distance: CGFloat) {
        if !mounted || drag == 0 { dismissKeyboard() }
        mounted = true
        drag = max(0, distance)
        arm(distance: drag, height: screenHeight)
    }

    private func settle(distance: CGFloat, velocity: CGFloat, height: CGFloat) {
        move(to: target(distance: distance + velocity * 0.15, height: height))
    }

    private func target(distance: CGFloat, height: CGFloat) -> PanelLevel {
        switch level {
        case .closed:
            distance > base(.peek, height) + Self.pushThreshold ? .full
                : distance > Self.pullThreshold ? .peek : .closed
        case .peek:
            distance > Self.pushThreshold ? .full : distance < -Self.pullThreshold ? .closed : .peek
        case .full:
            distance < -Self.pullThreshold ? .peek : .full
        }
    }

    /// 拖到“松手就换一级”的位置时轻震一下，同系统下拉刷新；往回退过阈值不震。
    private func arm(distance: CGFloat, height: CGFloat) {
        let next = target(distance: distance, height: height)
        defer { armed = next }
        guard next != armed, next != level, Self.depth(next) > Self.depth(armed) else { return }
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        buzzes += 1
    }

    private static func depth(_ level: PanelLevel) -> Int {
        switch level {
        case .closed: 0
        case .peek: 1
        case .full: 2
        }
    }

    private func move(to target: PanelLevel) {
        let landed = target == .full && items.isEmpty ? .peek : target
        withAnimation(reduceMotion ? .easeOut(duration: 0.15) : .spring(response: 0.38, dampingFraction: 0.86)) {
            level = landed
            drag = 0
            armed = landed
        }
    }

    private func openPanel() {
        mounted = true
        syncSelection()
        move(to: .peek)
    }

    private func dismissKeyboard() {
        UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
    }

    // MARK: - 选择

    private func syncSelection() {
        let available = items.map(\.tab)
        if let selection, available.contains(selection) { return }
        selection = selected.flatMap { available.contains($0) ? $0 : nil } ?? available.first
    }

    private func window() -> Set<PlacedTab> {
        guard let index = items.firstIndex(where: { $0.tab == (selection ?? selected) }) else { return [] }
        return Set([index - 1, index, index + 1].filter { items.indices.contains($0) }
            .map { PlacedTab.panel(scope: scope, tab: items[$0].tab) })
    }

    private func retainWindow() { webViews.retain(window()) }
}

extension ChatPanelHost {
    // MARK: - 视图

    private func chat(height: CGFloat) -> some View {
        content()
            // 拉开后聊天页不接点击：它的整屏底色背景还留在原位，会把面板整块盖住（房间有 WontonCanvas 的底色时实测）。
            // 露出的那一条由上面的 `strip` 单独接点按和拖动；收起时聊天页照常可点。
            .allowsHitTesting(level == .closed)
            .offset(y: sign * chatOffset(height))
            .background {
                if let pullAnchor {
                    RoomPullBinder(anchor: pullAnchor, pull: pull(height: height)).allowsHitTesting(false)
                }
            }
            .shadow(color: .black.opacity(level == .closed && drag == 0 ? 0 : 0.15), radius: 12)
            .accessibilityAction(named: Text(TabText.openPanel)) { openPanel() }
    }

    /// 露出的那条聊天页：点一下收起，往外推换级。单独一层放在聊天页上面，不靠它的位移决定命中区——
    /// 叠在聊天页上的点按区不跟着 `.offset` 走，面板会被整块盖住，第 1 级里点加号、圆点、网页都会被吃掉。
    private func strip(height: CGFloat) -> some View {
        ZStack(alignment: edge == .top ? .top : .bottom) {
            Color.clear
            if level == .peek {
                PanelGrabber()
            }
        }
        .contentShape(Rectangle())
        .frame(height: max(0, height - chatOffset(height)))
        .frame(maxHeight: .infinity, alignment: edge == .top ? .bottom : .top)
        .onTapGesture { move(to: .closed) }
        .gesture(
            // 按屏幕算：聊天页跟着手指动，按它自己的坐标算的话手指一直停在原处。
            DragGesture(minimumDistance: 4, coordinateSpace: .global)
                .onChanged {
                    drag = sign * $0.translation.height
                    arm(distance: drag, height: height)
                }
                .onEnded { value in
                    settle(distance: sign * value.translation.height,
                           velocity: sign * value.velocity.height, height: height)
                }
        )
    }

    @ViewBuilder private func pager(height: CGFloat, safe: EdgeInsets) -> some View {
        // 网页框一直是整屏高：主面板被聊天页盖住的是聊天页顶边以下，房间是滑出屏幕的那段。
        let covered = level == .full
            ? safe.bottom
            : max(0, height - chatOffset(height) - (edge == .top ? safe.top : 0))
        let top = max(0, safe.top - panelTop(height))
        ZStack(alignment: .bottom) {
            if items.isEmpty {
                PanelEmpty(add: { editing = true })
                    .frame(height: chatOffset(height) + (edge == .top ? safe.top : 0))
                    .frame(maxHeight: .infinity, alignment: .top)
            } else {
                pages(height: height, insetTop: top, insetBottom: covered)
            }
            if items.count > 1 || selectedIsURL {
                dots(safe: safe)
            }
        }
        .background(Color(colors.canvas))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chatPanel")
        .accessibilityHidden(level == .closed && drag == 0)
    }

    private var selectedIsURL: Bool {
        guard let tab = selection ?? selected, case .url = tab else { return false }
        return true
    }

    private func panelTop(_ height: CGFloat) -> CGFloat {
        edge == .top ? 0 : max(0, height - chatOffset(height))
    }

    private func pages(height: CGFloat, insetTop: CGFloat, insetBottom: CGFloat) -> some View {
        TabView(selection: Binding(get: { selection }, set: { selection = $0 })) {
            ForEach(items, id: \.tab) { item in
                page(item, height: height, insetTop: insetTop, insetBottom: insetBottom)
                    .tag(Optional(item.tab))
            }
        }
        .tabViewStyle(.page(indexDisplayMode: .never))
        // 整页时横滑交给网页（地图、noVNC、轮播），翻页靠小圆点。
        .scrollDisabled(level == .full)
    }

    private func page(_ item: PanelItem, height: CGFloat, insetTop: CGFloat, insetBottom: CGFloat) -> some View {
        let placed = PlacedTab.panel(scope: scope, tab: item.tab)
        let selectedPage = selection == item.tab
        let pageDirection: CGFloat = level == .full ? -1 : 1
        let pageEdge: PanelEdge = level == .full ? .top : edge
        let mode = TabPanelMode(
            expanded: level == .full,
            visible: level != .closed && selectedPage,
            insetTop: insetTop,
            insetBottom: insetBottom,
            pagePull: selectedPage && ((edge == .top && level == .peek) ||
                (edge == .bottom && (level == .peek || level == .full))),
            pageEdge: pageEdge,
            pageChanged: { distance in
                drag = pageDirection * distance
                arm(distance: drag, height: height)
            },
            pageEnded: { distance, velocity in
                settle(
                    distance: pageDirection * distance,
                    velocity: pageDirection * velocity,
                    height: height
                )
            },
            close: { move(to: .closed) },
            edit: { editing = true }
        )
        return TabPageView(
            tabs: tabs, home: home, plugins: plugins, chat: chat, appearance: appearance,
            tab: item.tab, placed: placed, store: webViews, page: item.page, isOverlay: false,
            keepAlive: window().contains(placed),
            draft: draft, open: open, pendingCards: pendingCards, openCardRoom: openCardRoom,
            panel: mode, remove: { remove(item.tab) }
        )
        .id(item.tab)
    }

    /// 小圆点表示第几页：第 1 级在露出的聊天条上面，第 2 级在 Home 条上面（ADR-0052）。
    private func dots(safe: EdgeInsets) -> some View {
        let bottom: CGFloat = if level == .full {
            safe.bottom + WontonTheme.spacing.s16
        } else if edge == .top {
            screenHeight - chatOffset(screenHeight) - safe.top + WontonTheme.spacing.s8
        } else {
            screenHeight - WontonTheme.spacing.s8
        }
        return PanelDots(items: items, selection: $selection)
            .padding(.bottom, bottom)
    }

    /// 面板收着时把红点画在聊天页挨着面板的那条边上。
    @ViewBuilder private func attention(height: CGFloat) -> some View {
        if level == .peek, items.contains(where: \.attention) {
            let edgeY = edge == .top
                ? chatOffset(height) + safeTop + 8
                : max(0, height - chatOffset(height) - 16)
            Circle()
                .fill(Color(colors.danger))
                .frame(width: 8, height: 8)
                .accessibilityLabel(TabText.panelAttention)
                .padding(.top, edgeY)
                .frame(maxHeight: .infinity, alignment: .top)
                .frame(maxWidth: .infinity)
                .allowsHitTesting(false)
        }
    }

    private var editorCandidates: [PanelCandidate] {
        let inPanel = Set(items.map(\.tab))
        let pluginPages = tabs.viewModel.panelCandidates.compactMap(\.panelCandidate)
        var all = pluginPages
        if case .room = scope {
            all += tabs.viewModel.panel.map { PanelCandidate(tab: $0.tab, title: $0.page.title, icon: nil) }
        }
        return all.filter { !inPanel.contains($0.tab) }
    }

    private var editor: some View {
        PanelEditor(
            items: items,
            candidates: editorCandidates,
            plugins: plugins,
            add: { tabs.send(.addPanelPage(scope: scope, tab: $0)) },
            remove: { remove($0) },
            move: { tab, index in tabs.send(.movePanelPage(scope: scope, tab: tab, index: UInt32(index))) },
            addURL: { tabs.send(.addPanelPage(scope: scope, tab: .url(url: $0))) }
        )
    }
}

private struct PanelGrabber: View {
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Capsule()
            .fill(Color(colors.separator))
            .frame(width: WontonTheme.size.grabberWidth, height: WontonTheme.size.grabberHeight)
            .padding(.vertical, WontonTheme.spacing.s8)
            .accessibilityHidden(true)
    }
}

private struct PanelDots: View {
    let items: [PanelItem]
    @Binding var selection: TabRef?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            if let host = sourceHost {
                HStack(spacing: WontonTheme.spacing.s4) {
                    WontonIcon.globe.view(size: WontonTheme.size.iconTiny)
                    Text(host)
                        .wontonText(WontonTheme.typography.caption)
                        .lineLimit(1)
                        .frame(maxWidth: 130, alignment: .leading)
                }
                .accessibilityLabel(host)
                .accessibilityIdentifier("panelSource")
            }
            if items.count > 1 {
                ForEach(items, id: \.tab) { item in
                    Circle()
                        .fill(Color(colors.textPrimary)
                            .opacity((selection ?? items.first?.tab) == item.tab ? 0.8 : 0.25))
                        .frame(width: 7, height: 7)
                        .overlay(alignment: .topTrailing) {
                            if item.attention {
                                Circle()
                                    .fill(Color(colors.danger))
                                    .frame(width: 5, height: 5)
                                    .offset(x: 2, y: -2)
                            }
                        }
                        .frame(width: 20, height: 20)
                        .contentShape(Rectangle())
                        .onTapGesture { withAnimation { selection = item.tab } }
                        .accessibilityLabel(item.page.title)
                        .accessibilityAddTraits(
                            (selection ?? items.first?.tab) == item.tab ? .isSelected : AccessibilityTraits()
                        )
                }
            }
        }
        .padding(.horizontal, WontonTheme.spacing.s12)
        .padding(.vertical, WontonTheme.spacing.s8)
        .background(.ultraThinMaterial, in: Capsule())
        .accessibilityIdentifier("panelDots")
    }

    private var sourceHost: String? {
        guard let tab = selection ?? items.first?.tab, case let .url(value) = tab else { return nil }
        return URL(string: value)?.host()
    }
}

private struct PanelEmpty: View {
    let add: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            Spacer()
            Button(action: add) {
                WontonIcon.add.view(size: WontonTheme.size.icon)
                    .foregroundStyle(Color(colors.textPrimary))
                    .frame(width: WontonTheme.size.detailIcon, height: WontonTheme.size.detailIcon)
                    .background(.ultraThinMaterial, in: Circle())
                    .overlay { Circle().strokeBorder(Color(colors.separator), lineWidth: WontonTheme.size.hairline) }
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(TabText.addPanelPages)
            .accessibilityIdentifier("panelAdd")
            Text(TabText.addPanelPages)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
                .accessibilityHidden(true)
            Spacer()
        }
        .frame(maxWidth: .infinity)
        .contentShape(Rectangle())
    }
}

#if DEBUG
/// 模拟器没有触觉，每震一次在屏幕中间闪一下，看得出在哪里震。
private struct BuzzMark: View {
    let count: Int
    @State private var visible = false

    var body: some View {
        Image(systemName: "iphone.radiowaves.left.and.right")
            .font(.system(size: 28, weight: .semibold))
            .padding(14)
            .background(.ultraThinMaterial, in: Circle())
            .opacity(visible ? 1 : 0)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .allowsHitTesting(false)
            .onChange(of: count) {
                visible = true
                withAnimation(.easeOut(duration: 0.5).delay(0.25)) { visible = false }
            }
    }
}
#endif

/// 把房间包进从下方出来的面板。房间的面板按房间给，页面来自每一批时间线（ADR-0052）。
struct RoomPanelHost<Content: View>: View {
    let roomId: String
    let panel: [PanelItem]
    let selected: TabRef?
    let blocked: Bool
    let pullAnchor: PullAnchor?
    @ViewBuilder let content: () -> Content
    @Environment(\.panelDeps) private var deps

    var body: some View {
        if let deps {
            let scope = PanelScope.room(roomId: roomId)
            ChatPanelHost(
                tabs: deps.tabs, home: deps.home, plugins: deps.plugins, chat: deps.chat,
                appearance: deps.appearance, scope: scope, edge: .bottom, items: panel, selected: selected,
                blocked: blocked, draft: deps.draft, open: deps.open, pendingCards: deps.pendingCards,
                openCardRoom: deps.openCardRoom,
                select: { deps.tabs.send(.selectPanelPage(scope: scope, tab: $0)) },
                remove: { deps.tabs.send(.removePanelPage(scope: scope, tab: $0)) },
                move: { deps.tabs.send(.movePanelPage(scope: scope, tab: $0, index: UInt32($1))) },
                pullAnchor: pullAnchor,
                content: { content() }
            )
        } else {
            content()
        }
    }
}
