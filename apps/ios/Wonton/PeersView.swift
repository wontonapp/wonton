import SwiftUI
import WontonClient

/// “其他服务器”（ADR-0050）：配对过的服务器、发来的请求、出示配对码和扫对方的码。状态和动作都在客户端核心，
/// 这里只渲染 ViewModel、把操作发回核心。
struct PeersView: View {
    @State private var model: PeersModel
    /// 好友请求也在这里露出来（ADR-0051）：请求就是 Matrix 私聊邀请，和配对请求不在一个名单里。
    @State private var friends: FriendsModel
    @Environment(\.wontonColors) private var colors
    @State private var showsAdd = false
    /// 主人要求出码：码到了才弹层。推入动画还没结束或上一个弹层还在收起时就开，这次呈现会被系统丢掉，
    /// 绑定却留在 true 上，之后再点也不弹（模拟器实测的“弹层不出来”就是这个）；等核心把码送回来再开，动画早就结束了。
    @State private var wantsCode = false
    @State private var showsScanner = false
    /// 删除的确认框挂在整页上：挂在行上，滑动删除后行没了，弹窗跟着消失。
    @State private var removing: PeerRow?
    @State private var showsRejected = false
    /// App 链接带过来的“滚到请求”；只在请求标过之后滚一次。
    private let focusRequests: Int
    @State private var scrolledToRequests = false

    init(core: ClientCore, focusRequests: Int = 0) {
        _model = State(initialValue: PeersModel(core: core))
        _friends = State(initialValue: FriendsModel(core: core))
        self.focusRequests = focusRequests
    }

    var body: some View {
        ScrollViewReader { proxy in
            content(proxy)
                .wontonPushedScreen(String(localized: .Peers3.peersTitle), usesSystemToolbar: true) {
                    WontonIconButton(icon: .add, label: String(localized: .Peers3.peersAdd)) { showsAdd = true }
                }
                .task { await model.observe() }
                .task { await friends.observe() }
                .refreshable {
                    await model.refresh()
                    await friends.refresh()
                }
                .task(id: focusRequests) { scrollToRequests(proxy) }
                .onChange(of: model.viewModel.requests.count) { scrollToRequests(proxy) }
                .onChange(of: model.viewModel.state) { _, state in
                    guard case let .failed(error, _) = state else { return }
                    if error == .requestNotFound { model.send(.peers(action: .refreshPeers)) }
                }
                .wontonActionSheet(
                    isPresented: $showsAdd, usesSystemPresentation: true, actions: addActions
                )
                .wontonSheet(
                    isPresented: Binding(
                        get: { wantsCode && model.viewModel.code != nil },
                        set: { if !$0 { hideCode() } }
                    ),
                    title: String(localized: .Peers1.peersShowCodeTitle),
                    usesSystemPresentation: true
                ) { _ in
                    PeersCodePanel(viewModel: model.viewModel) { model.send(.peers(action: .resetPeerCode)) }
                }
                .fullScreenCover(isPresented: $showsScanner) {
                    NavigationStack {
                        PeersScannerView(model: model)
                            .wontonModalScreen(String(localized: .Peers2.peersScanCodeTitle), usesSystemToolbar: true)
                    }
                }
                .peersRemoveDialog($removing, model: model)
                .peersRejectionDialog(model: model)
                .friendsRejectionDialog(model: friends)
                .onDisappear {
                    // 离开页面必须让核心停下取码：出码页开着时它每 3 秒取一次列表。
                    model.send(.peers(action: .hidePeerCode))
                }
                .onChange(of: model.viewModel.state) { _, state in
                    // 出码的请求失败了：码不会来了，收回请求，失败原因在页面上说。
                    if case .failed = state, model.viewModel.code == nil, wantsCode { hideCode() }
                }
        }
    }

    @ViewBuilder private func content(_ proxy: ScrollViewProxy) -> some View {
        if model.viewModel.peers.isEmpty, model.viewModel.requests.isEmpty, model.viewModel.rejected.isEmpty,
           friends.viewModel.incoming.isEmpty {
            emptyState
        } else {
            list(proxy)
        }
    }

    /// 空列表：正在取就说在取；取到了就是一句话加两个按钮；取失败整页说明原因。
    @ViewBuilder private var emptyState: some View {
        switch model.viewModel.state {
        case .loading:
            LoadingView()
        case .ready:
            ScrollView {
                PeersEmpty(
                    showCode: showCode,
                    scan: { showsScanner = true }
                )
            }
        case let .failed(error, message):
            ProblemView(problem: peersProblem(error, message: message), retry: retry)
        }
    }

    private func list(_ proxy: ScrollViewProxy) -> some View {
        List {
            if case .loading = model.viewModel.state {
                HStack {
                    Spacer()
                    ProgressView()
                    Spacer()
                }
                .padding(.vertical, WontonTheme.spacing.s12)
                .listRowSeparator(.hidden)
            }
            if case let .failed(error, message) = model.viewModel.state {
                ProblemRow(problem: peersProblem(error, message: message), retry: retry)
                    .listRowSeparator(.hidden)
            }
            if !friends.viewModel.incoming.isEmpty {
                // 好友请求排最前：对方正在等答复，比配对请求更要紧。
                Section {
                    ForEach(friends.viewModel.incoming, id: \.roomId) { request in
                        FriendRequestRowView(
                            request: request,
                            accept: { friends.send(.friends(action: .approveFriendRequest(roomId: request.roomId))) },
                            decline: { friends.send(.friends(action: .rejectFriendRequest(roomId: request.roomId))) }
                        )
                        .listRowBackground(Color(colors.surfaceSubtle))
                        .listRowSeparatorTint(Color(colors.separator))
                    }
                } header: {
                    Text(.Friends1.friendsRequests)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            }
            if !model.viewModel.requests.isEmpty {
                Section {
                    ForEach(model.viewModel.requests, id: \.id) { request in
                        PeerRequestRowView(
                            request: request,
                            approve: { model.send(.peers(action: .approvePeerRequest(id: request.id))) },
                            reject: { model.send(.peers(action: .rejectPeerRequest(id: request.id))) }
                        )
                        .listRowBackground(Color(colors.surfaceSubtle))
                        .listRowSeparatorTint(Color(colors.separator))
                    }
                } header: {
                    Text(.Peers3.peersRequests)
                        .foregroundStyle(Color(colors.textSecondary))
                        .id("requests")
                }
            }
            ForEach(model.viewModel.peers, id: \.serverName) { row in
                PeerRowView(row: row, remove: { requestRemove(row) })
                    // 行底和设置页其他列表一样用 surfaceSubtle；分隔线颜色也对齐令牌（默认的拿不到深浅色）。
                    .listRowBackground(Color(colors.surfaceSubtle))
                    .listRowSeparatorTint(Color(colors.separator))
                    .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                        Button(role: .destructive) { requestRemove(row) } label: {
                            Label(String(localized: .Common.commonRemove), systemImage: "trash")
                        }
                    }
            }
            if !model.viewModel.rejected.isEmpty {
                Section {
                    if showsRejected {
                        ForEach(model.viewModel.rejected, id: \.id) { row in
                            RejectedRequestRowView(row: row) {
                                model.send(.peers(action: .unrejectPeerRequest(id: row.id)))
                            }
                            .listRowBackground(Color(colors.surfaceSubtle))
                            .listRowSeparatorTint(Color(colors.separator))
                        }
                    }
                } header: {
                    Button {
                        withAnimation { showsRejected.toggle() }
                    } label: {
                        HStack(spacing: WontonTheme.spacing.s4) {
                            Text(.Peers3.peersRejected)
                            WontonIcon.forward.view(size: WontonTheme.size.iconTiny)
                                .rotationEffect(showsRejected ? .degrees(90) : .zero)
                        }
                        .foregroundStyle(Color(colors.textSecondary))
                    }
                }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
    }
}

private extension PeersView {
    private var addActions: [WontonSheetAction] {
        [
            WontonSheetAction(title: String(localized: .Peers2.peersShowCodeButton), icon: .qrCode) {
                showCode()
            },
            WontonSheetAction(title: String(localized: .Peers2.peersScanCodeButton), icon: .expand) {
                showsScanner = true
            }
        ]
    }

    /// 出示我的码：把请求发给核心，码到了由上面的派生绑定弹层。先按下去、下一个 runloop 再抬起来，让呈现
    /// 请求离开点按自己的更新事务（真机上空页面按钮点了不弹，就是呈现被丢、绑定卡在 true；菜单等面板收起
    /// 才执行，没踩到）。
    private func showCode() {
        wantsCode = false
        Task { @MainActor in
            wantsCode = true
            model.send(.peers(action: .showPeerCode))
        }
    }

    /// 关掉出码页：核心不再快速取码。
    private func hideCode() {
        guard wantsCode else { return }
        wantsCode = false
        model.send(.peers(action: .hidePeerCode))
    }

    /// 取消自己发出、对方还没处理的请求和清掉被拒的记录，直接做（和 Android 一致）；移除配对两边都会停，才要确认。
    private func requestRemove(_ row: PeerRow) {
        if row.status == .awaitingApproval || row.status == .declined {
            model.send(.peers(action: .removePeer(serverName: row.serverName)))
        } else {
            removing = row
        }
    }

    /// 失败后重试：扫码没扫成的回到扫码，其余重取列表。
    private func retry() {
        if model.failedScan {
            showsScanner = true
        } else {
            model.send(.peers(action: .refreshPeers))
        }
    }

    /// 从 App 链接进来时滚到“请求”：请求晚于页面到达，等标过之后滚一次。
    private func scrollToRequests(_ proxy: ScrollViewProxy) {
        guard focusRequests > 0, !scrolledToRequests, !model.viewModel.requests.isEmpty else { return }
        scrolledToRequests = true
        withAnimation { proxy.scrollTo("requests", anchor: .top) }
    }

    private func peersProblem(_ error: PeerError, message: String) -> Problem {
        // TransportUnavailable 和插件、聊天那边共用同一句；连不上家里的服务器时列表取不到，是这里最常见的原因。
        let title: LocalizedStringResource = switch error {
        case .codeInvalid: .Peers0.peersErrorCodeInvalid
        case .codeRevoked: .Peers1.peersErrorCodeRevoked
        case .tooManyRequests: .Peers0.peersErrorTooManyRequests
        case .requestNotFound: .Peers1.peersNoticeRequestNotFound
        case .peerUnreachable: .Peers1.peersErrorPeerUnreachable
        case .peerRejected: .Peers2.peersErrorPeerRejected
        case .alreadyPaired: .Peers0.peersErrorAlreadyPaired
        case .selfPairing: .Peers1.peersErrorSelfPairing
        case .notPaired: .Peers2.peersErrorNotPaired
        case .unauthenticated: .Peers0.peersErrorUnauthenticated
        case .transportUnavailable: .Problem2.problemTransportUnavailableTitle
        case .protocol, .other: .Peers1.peersErrorOther
        }
        return Problem(title: String(localized: title), detail: message)
    }
}

/// 页面的状态：核心的 `PeersViewModel`，加上“最近一次失败是不是扫码”和核心当场拒绝的操作。
@MainActor
@Observable
final class PeersModel {
    private(set) var viewModel: PeersViewModel
    /// 最近一次失败是扫码没扫成：重试回到扫码，而不是重取列表。
    private(set) var failedScan = false
    /// 核心拒绝上一个操作的原因，由界面弹窗展示。
    var rejection: Problem?

    private let core: ClientCore

    init(core: ClientCore) {
        self.core = core
        viewModel = core.peers()
    }

    /// 进页面时取一次列表，之后跟着核心的推送走；页面消失时订阅结束。
    func observe() async {
        send(.peers(action: .refreshPeers))
        for await viewModel in core.subscribePeers().updates() {
            // Loading 只来自取列表：出现它说明上一次扫码已经处理完，重试回到刷新。
            if case .loading = viewModel.state { failedScan = false }
            self.viewModel = viewModel
        }
    }

    func send(_ action: Action) {
        if case .peers(action: .scanPeerCode) = action { failedScan = true }
        do {
            try core.dispatch(action: action)
        } catch {
            rejection = error.problem
        }
    }

    /// 下拉刷新：等核心把状态从 Loading 换走，圈才收起来。
    func refresh() async {
        send(.peers(action: .refreshPeers))
        guard rejection == nil else { return }
        for await viewModel in core.subscribePeers().updates() {
            if case .loading = viewModel.state { continue }
            return
        }
    }
}

/// 出码弹层的内容：正常进来码已经在 ViewModel 里；万一在呈现和渲染之间被核心清掉，转一圈等绑定关停。
private struct PeersCodePanel: View {
    let viewModel: PeersViewModel
    let reset: () -> Void
    @State private var confirmingReset = false

    var body: some View {
        Group {
            if let code = viewModel.code {
                PeersCode(code: code) { confirmingReset = true }
            } else {
                ProgressView()
                    .frame(maxWidth: .infinity, minHeight: WontonTheme.size.detailIcon)
                    .padding(.vertical, WontonTheme.spacing.s24)
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .wontonDialog(
            String(localized: .Peers2.peersResetTitle),
            message: String(localized: .Peers2.peersResetMessage),
            isPresented: $confirmingReset,
            actions: [
                WontonDialogAction(title: String(localized: .Common.commonCancel)) { confirmingReset = false },
                WontonDialogAction(title: String(localized: .Peers1.peersReset), kind: .danger) {
                    reset()
                    confirmingReset = false
                }
            ]
        )
    }
}

/// 配对二维码和分享、重置。码不过期，重置以后旧的作废。
private struct PeersCode: View {
    let code: PeerCode
    let reset: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s16) {
            if let image = QRImage.make(code.code) {
                Image(uiImage: image)
                    .interpolation(.none)
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: 260)
                    .clipShape(RoundedRectangle(cornerRadius: WontonTheme.radii.control))
                    .accessibilityLabel(String(localized: .Peers0.peersCodeQrLabel))
            }
            HStack(spacing: WontonTheme.spacing.s8) {
                ShareLink(item: code.code) {
                    Text(.Peers2.peersShare).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle())
                Button(.Peers1.peersReset, action: reset)
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, WontonTheme.spacing.s24)
    }
}

extension View {
    /// 移除一台服务器的确认框：挂在整页上，滑动删除和行内按钮只负责把它带出来。取消请求和清被拒记录
    /// 不经过这里，直接发动作（见 `requestRemove`）。
    func peersRemoveDialog(_ removing: Binding<PeerRow?>, model: PeersModel) -> some View {
        let row = removing.wrappedValue
        let name = row.map { $0.ownerDisplayName.isEmpty ? $0.serverName : $0.ownerDisplayName } ?? ""
        return wontonDialog(
            String(localized: .Peers1.peersRemoveTitle(name)),
            message: String(localized: .Peers0.peersRemoveMessage),
            isPresented: Binding(
                get: { removing.wrappedValue != nil }, set: { if !$0 { removing.wrappedValue = nil } }
            ),
            actions: [
                WontonDialogAction(title: String(localized: .Common.commonCancel)) { removing.wrappedValue = nil },
                WontonDialogAction(title: String(localized: .Common.commonRemove), kind: .danger) {
                    if let row { model.send(.peers(action: .removePeer(serverName: row.serverName))) }
                    removing.wrappedValue = nil
                }
            ]
        )
    }

    /// 核心当场拒绝了一个操作（比如列表还没就绪）：说清原因就好。
    func peersRejectionDialog(model: PeersModel) -> some View {
        wontonDialog(problem: model.rejection, isPresented: Binding(
            get: { model.rejection != nil }, set: { if !$0 { model.rejection = nil } }
        ))
    }
}
