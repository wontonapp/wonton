import SwiftUI
import WontonClient

/// 好友（ADR-0051）：请求就是 Matrix 私聊邀请，同意以后成为好友。状态和动作都在客户端核心，
/// 这里只渲染 ViewModel、把操作发回核心。
struct FriendsView: View {
    @State private var model: FriendsModel
    @Environment(\.wontonColors) private var colors
    @State private var showsAdd = false
    @State private var showsScanner = false
    @State private var showsMyCode = false
    /// 扫到或粘贴的原文：扫码页收起之后才交给核心，失败的弹窗才挂得上这一页。
    @State private var scannedCode: String?
    @State private var newUserId = ""
    @FocusState private var addFocused: Bool
    /// 删除好友的确认框挂在整页上：挂在行上，滑动删除后行没了，弹窗跟着消失。
    @State private var removing: FriendRow?
    @State private var serverName = ""

    init(core: ClientCore) {
        _model = State(initialValue: FriendsModel(core: core))
    }

    var body: some View {
        content
            .wontonPushedScreen(String(localized: .Friends2.friendsTitle), usesSystemToolbar: true) {
                WontonIconButton(icon: .add, label: String(localized: .Friends1.friendsAdd)) { showsAdd = true }
            }
            .task { await model.observe() }
            .refreshable { await model.refresh() }
            .wontonSheet(
                isPresented: $showsAdd,
                title: String(localized: .Friends1.friendsAdd), usesSystemPresentation: true
            ) { close in
                addPanel(close)
            }
            .fullScreenCover(isPresented: $showsScanner, onDismiss: sendScannedCode) {
                NavigationStack {
                    FriendScannerView { code in
                        scannedCode = code
                        showsScanner = false
                    }
                    .wontonModalScreen(String(localized: .Friends1.friendsScanCode), usesSystemToolbar: true)
                }
            }
            .wontonSheet(
                isPresented: Binding(
                    get: { showsMyCode && model.viewModel.myLink != nil }, set: { if !$0 { showsMyCode = false } }
                ),
                title: String(localized: .Friends3.friendsMyCode),
                usesSystemPresentation: true
            ) { _ in
                if let link = model.viewModel.myLink { MyFriendCodePanel(link: link) }
            }
            .friendsRemoveDialog($removing, model: model)
            .friendsRejectionDialog(model: model)
    }

    @ViewBuilder private var content: some View {
        if model.viewModel.friends.isEmpty, model.viewModel.incoming.isEmpty, model.viewModel.outgoing.isEmpty,
           model.viewModel.blocked.isEmpty {
            emptyState
        } else {
            list
        }
    }

    /// 空页：正在取就转圈；取到了就是一句话加加好友；取失败整页说明原因。
    @ViewBuilder private var emptyState: some View {
        switch model.viewModel.state {
        case .loading:
            LoadingView()
        case .ready:
            ScrollView { FriendsEmpty { showsAdd = true } }
        case let .failed(problem):
            ProblemView(problem: friendsProblem(problem), retry: retry)
        case .other:
            ScrollView { FriendsEmpty { showsAdd = true } }
        }
    }

    private var list: some View {
        List {
            statusRows
            incomingSection
            outgoingSection
            friendsSection
            blockedSection
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
    }

    /// 正在取和失败：不进任何一节，列表还在时就地提示。
    @ViewBuilder private var statusRows: some View {
        if case .loading = model.viewModel.state {
            HStack {
                Spacer()
                ProgressView()
                Spacer()
            }
            .padding(.vertical, WontonTheme.spacing.s12)
            .listRowSeparator(.hidden)
        }
        if case let .failed(problem) = model.viewModel.state {
            ProblemRow(problem: friendsProblem(problem), retry: retry)
                .listRowSeparator(.hidden)
        }
    }

    /// 收到的请求。普通私聊邀请也在这里，标记由行自己按类别加。
    @ViewBuilder private var incomingSection: some View {
        if !model.viewModel.incoming.isEmpty {
            Section {
                ForEach(model.viewModel.incoming, id: \.roomId) { request in
                    FriendRequestRowView(
                        request: request,
                        accept: { model.send(.approveFriendRequest(roomId: request.roomId)) },
                        decline: { model.send(.rejectFriendRequest(roomId: request.roomId)) }
                    )
                    .listRowBackground(Color(colors.surfaceSubtle))
                    .listRowSeparatorTint(Color(colors.separator))
                }
            } header: {
                sectionHeader(.Friends2.friendsIncoming)
            }
        }
    }

    /// 自己发出、还没被处理的请求。
    @ViewBuilder private var outgoingSection: some View {
        if !model.viewModel.outgoing.isEmpty {
            Section {
                ForEach(model.viewModel.outgoing, id: \.roomId) { request in
                    OutgoingFriendRequestRowView(request: request) {
                        model.send(.cancelFriendRequest(roomId: request.roomId))
                    }
                    .listRowBackground(Color(colors.surfaceSubtle))
                    .listRowSeparatorTint(Color(colors.separator))
                }
            } header: {
                sectionHeader(.Friends0.friendsOutgoing)
            }
        }
    }

    private var friendsSection: some View {
        Section {
            ForEach(model.viewModel.friends, id: \.userId) { row in
                FriendRowView(
                    row: row,
                    remove: { removing = row },
                    block: { model.send(.blockFriend(target: .user(userId: row.userId), scope: .invitations)) }
                )
                .listRowBackground(Color(colors.surfaceSubtle))
                .listRowSeparatorTint(Color(colors.separator))
                .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                    Button(role: .destructive) { removing = row } label: {
                        Label(String(localized: .Friends2.friendsRemove), systemImage: "trash")
                    }
                }
            }
            if model.viewModel.friends.isEmpty {
                Text(.Friends1.friendsEmpty)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
                    .listRowBackground(Color(colors.surfaceSubtle))
                    .listRowSeparatorTint(Color(colors.separator))
            }
        } header: {
            sectionHeader(.Friends1.friendsList)
        }
    }

    /// 已屏蔽：个人的邀请屏蔽随时能撤；能管整台服务器时再给输入框和服务器列表。
    private var blockedSection: some View {
        Section {
            if model.viewModel.canBlockFederation {
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                    // verbatim：占位符不是链接，也不是已经填好的内容。
                    TextField(String(localized: .Friends0.friendsBlockServerField), text: $serverName,
                              prompt: Text(verbatim: "example.org"))
                        .keyboardType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .wontonField()
                    Button(.Friends3.friendsBlockServer) { blockServer() }
                        .buttonStyle(WontonSmallButtonStyle())
                        .disabled(normalizedServerName.isEmpty)
                }
                .padding(.vertical, WontonTheme.spacing.s8)
                .listRowBackground(Color(colors.surfaceSubtle))
                .listRowSeparatorTint(Color(colors.separator))
            }
            ForEach(Array(model.viewModel.blocked.enumerated()), id: \.offset) { _, block in
                BlockedFriendRowView(block: block) {
                    model.send(.unblockFriend(target: block.target, scope: block.scope))
                }
                .listRowBackground(Color(colors.surfaceSubtle))
                .listRowSeparatorTint(Color(colors.separator))
            }
        } header: {
            sectionHeader(.Friends2.friendsBlocked)
        }
    }

    @ViewBuilder private func sectionHeader(_ title: LocalizedStringResource) -> some View {
        Text(title)
            .foregroundStyle(Color(colors.textSecondary))
    }
}

private extension FriendsView {
    private var normalizedUserId: String {
        newUserId.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// 用户 ID 和 matrix.to 链接都收，认不认得由核心判（不是用户的报输入非法）。
    private var canSubmitAdd: Bool {
        !normalizedUserId.isEmpty
    }

    private var normalizedServerName: String {
        serverName.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func addPanel(_ close: @escaping (@escaping () -> Void) -> Void) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            addForm(close)
            ForEach(addActions.indices, id: \.self) { index in
                WontonSheetActionRow(action: addActions[index]) { close(addActions[index].perform) }
            }
        }
    }

    /// 表单下面的两条退路：扫对方的码；出示自己的码要等核心给出自己的链接。
    private var addActions: [WontonSheetAction] {
        var actions = [WontonSheetAction(title: String(localized: .Friends1.friendsScanCode), icon: .expand) {
            showsScanner = true
        }]
        if model.viewModel.myLink != nil {
            actions.append(WontonSheetAction(title: String(localized: .Friends3.friendsMyCode), icon: .qrCode) {
                showsMyCode = true
            })
        }
        return actions
    }

    private func addForm(_ close: @escaping (@escaping () -> Void) -> Void) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            TextField(String(localized: .Friends2.friendsAddField), text: $newUserId,
                      prompt: Text(verbatim: "@bob:example.org"))
                .keyboardType(.asciiCapable)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .submitLabel(.send)
                .focused($addFocused)
                .onSubmit { submitAdd(close) }
                .wontonField()
            Button {
                submitAdd(close)
            } label: {
                Text(.Friends3.friendsAddSubmit).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle())
            .disabled(!canSubmitAdd)
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.vertical, WontonTheme.spacing.s16)
        .task {
            newUserId = ""
            addFocused = true
        }
    }

    private func sendScannedCode() {
        guard let code = scannedCode else { return }
        scannedCode = nil
        model.send(.addFriend(userId: code))
    }

    /// 发出去就收起面板；格式不对由核心回来时说。
    private func submitAdd(_ close: @escaping (@escaping () -> Void) -> Void) {
        guard canSubmitAdd else { return }
        model.send(.addFriend(userId: normalizedUserId))
        close {}
    }

    private func blockServer() {
        let name = normalizedServerName
        guard !name.isEmpty else { return }
        model.send(.blockFriend(target: .server(serverName: name), scope: .federation))
        serverName = ""
    }

    private func retry() {
        model.send(.refreshFriends)
    }

    /// 失败原因按类别写人话；限频把服务器给的秒数带上，技术细节收在下面。
    private func friendsProblem(_ problem: FriendProblem) -> Problem {
        let detail = friendProblemDetail(problem)
        switch problem.reason {
        case .invalidUserId:
            return Problem(title: String(localized: .Friends3.friendsErrorInvalidUserId),
                           nextStep: String(localized: .Friends0.friendsErrorInvalidUserIdNextStep), detail: detail)
        case .requestNotFound:
            return Problem(title: String(localized: .Friends0.friendsErrorRequestNotFound),
                           nextStep: String(localized: .Friends0.friendsErrorRequestNotFoundNextStep), detail: detail)
        case .rateLimited:
            let wait = problem.retryAfterMs.map { max(1, Int((Double($0) / 1000).rounded(.up))) } ?? 60
            return Problem(title: String(localized: .Friends0.friendsErrorRateLimited),
                           nextStep: String(localized: .Friends0.friendsErrorRateLimitedNextStep(wait)), detail: detail)
        case .unauthorized:
            return Problem(title: String(localized: .Problem3.problemUnauthorizedTitle),
                           nextStep: String(localized: .Problem2.problemUnauthorizedNextStep), detail: detail)
        case .forbidden:
            return Problem(title: String(localized: .Friends1.friendsErrorForbidden),
                           nextStep: String(localized: .Friends1.friendsErrorForbiddenNextStep), detail: detail)
        case .transportUnavailable:
            return Problem(title: String(localized: .Problem2.problemTransportUnavailableTitle),
                           nextStep: String(localized: .Problem3.problemTransportUnavailableNextStep), detail: detail)
        case .upstream:
            return Problem(title: String(localized: .Problem2.problemUpstreamTitle),
                           nextStep: String(localized: .Problem2.problemUpstreamNextStep), detail: detail)
        case .other:
            return Problem(title: String(localized: .Problem1.problemGenericTitle),
                           nextStep: String(localized: .Problem2.problemUpstreamNextStep), detail: detail)
        }
    }
}

/// 页面的状态：核心的 `FriendsViewModel`，加核心当场拒绝的操作。
@MainActor
@Observable
final class FriendsModel {
    private(set) var viewModel: FriendsViewModel
    /// 核心拒绝上一个操作的原因，由界面弹窗展示。
    var rejection: Problem?
    private let core: ClientCore

    init(core: ClientCore) {
        self.core = core
        viewModel = core.friends()
    }

    /// 进页面时取一次列表，之后跟着核心的推送走；页面消失时订阅结束。
    func observe() async {
        send(.refreshFriends)
        for await viewModel in core.subscribeFriends().updates() {
            self.viewModel = viewModel
        }
    }

    func send(_ action: Action) {
        do {
            try core.dispatch(action: action)
        } catch {
            rejection = error.problem
        }
    }

    /// 下拉刷新：等核心把状态从 Loading 换走，圈才收起来。
    func refresh() async {
        send(.refreshFriends)
        guard rejection == nil else { return }
        for await viewModel in core.subscribeFriends().updates() {
            if case .loading = viewModel.state { continue }
            return
        }
    }
}

/// 核心给的 HTTP 状态和 Matrix 错误码：只在排查时看。
private func friendProblemDetail(_ problem: FriendProblem) -> String? {
    var lines: [String] = []
    if let status = problem.httpStatus { lines.append("HTTP \(status)") }
    if let errcode = problem.matrixErrcode { lines.append(errcode) }
    return lines.isEmpty ? nil : lines.joined(separator: " ")
}

extension View {
    /// 删除好友的确认框：挂在整页上，滑动删除和行内按钮只负责把它带出来。
    func friendsRemoveDialog(_ removing: Binding<FriendRow?>, model: FriendsModel) -> some View {
        let row = removing.wrappedValue
        return wontonDialog(
            String(localized: .Friends3.friendsRemoveTitle(friendName(row?.displayName, userId: row?.userId ?? ""))),
            message: String(localized: .Friends3.friendsRemoveMessage),
            isPresented: Binding(
                get: { removing.wrappedValue != nil }, set: { if !$0 { removing.wrappedValue = nil } }
            ),
            actions: [
                WontonDialogAction(title: String(localized: .Common.commonCancel)) { removing.wrappedValue = nil },
                WontonDialogAction(title: String(localized: .Friends2.friendsRemove), kind: .danger) {
                    if let row { model.send(.removeFriend(userId: row.userId)) }
                    removing.wrappedValue = nil
                }
            ]
        )
    }

    /// 核心当场拒绝了一个操作（比如列表还没就绪）：说清原因就好。
    func friendsRejectionDialog(model: FriendsModel) -> some View {
        wontonDialog(problem: model.rejection, isPresented: Binding(
            get: { model.rejection != nil }, set: { if !$0 { model.rejection = nil } }
        ))
    }
}
