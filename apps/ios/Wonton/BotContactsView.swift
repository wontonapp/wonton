import SwiftUI
import WontonClient

/// 一个 Bot 的联系人（ADR-0045、ADR-0050）：同机和跨机是同一份列表。状态、失败原因都在客户端核心的
/// `BotContactsViewModel` 里，这里只渲染、把操作发回核心。“等对方”由核心自己跟，页面不轮询。
struct BotContactsView: View {
    @State private var model: BotContactsModel
    @Environment(\.wontonColors) private var colors
    @State private var showsAdd = false
    @State private var newUserId = ""
    @State private var addsPerson = false
    @FocusState private var addFocused: Bool
    /// 删除已生效联系人的确认框挂在整页上：挂在行上，滑动删除后行没了，弹窗跟着消失。
    @State private var removing: BotContact?

    init(core: ClientCore, bot: BotRef) {
        _model = State(initialValue: BotContactsModel(core: core, bot: bot))
    }

    var body: some View {
        content
            .wontonPushedScreen(String(localized: .contactsTitle)) {
                WontonIconButton(icon: .add, label: String(localized: .contactsAdd)) { showsAdd = true }
            }
            .task { await model.observe() }
            .refreshable { await model.refresh() }
            .wontonSheet(isPresented: $showsAdd, title: String(localized: .contactsAdd)) { close in
                addPanel(close)
            }
            .contactsRemoveDialog($removing, model: model)
            .wontonDialog(problem: model.rejection, isPresented: Binding(
                get: { model.rejection != nil }, set: { if !$0 { model.rejection = nil } }
            ))
    }

    @ViewBuilder private var content: some View {
        if model.contacts.isEmpty {
            emptyState
        } else {
            list
        }
    }

    /// 空页：正在取（或核心还看着别的 Bot）就转圈；取到了是一句话加添加；取失败整页说明原因。
    @ViewBuilder private var emptyState: some View {
        if let failure = model.failure {
            ProblemView(problem: contactsProblem(failure.error, message: failure.message)) { model.retry() }
        } else if model.isLoading {
            LoadingView()
        } else {
            ScrollView { BotContactsEmpty { showsAdd = true } }
        }
    }

    private var list: some View {
        List {
            if let failure = model.failure {
                ProblemRow(problem: contactsProblem(failure.error, message: failure.message)) { model.retry() }
                    .listRowSeparator(.hidden)
            }
            section(.contactsSectionRequests, model.contacts.filter { $0.state == .incoming }) { contact in
                IncomingContactRowView(
                    contact: contact,
                    approve: { model.send(.approveBotContact(bot: model.bot, userId: contact.userId)) },
                    reject: { model.send(.rejectBotContact(bot: model.bot, userId: contact.userId)) }
                )
            }
            section(.contactsSectionActive, model.contacts.filter { $0.state == .active }) { contact in
                ActiveContactRowView(contact: contact) { removing = contact }
                    .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                        Button(role: .destructive) { removing = contact } label: {
                            Label(String(localized: .contactsRemove), systemImage: "trash")
                        }
                    }
            }
            // 撤销请求、清掉被拒或查无此人的行都不用确认：对方那边什么都没成立。
            let sent = model.contacts.filter { $0.state != .incoming && $0.state != .active }
            section(.contactsSectionSent, sent) { contact in
                SentContactRowView(contact: contact) {
                    model.send(.removeBotContact(bot: model.bot, userId: contact.userId))
                }
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
    }

    /// 一节：没有行时整节不占位置。
    @ViewBuilder private func section<Row: View>(
        _ title: LocalizedStringResource,
        _ contacts: [BotContact],
        @ViewBuilder row: @escaping (BotContact) -> Row
    ) -> some View {
        if !contacts.isEmpty {
            Section {
                ForEach(contacts, id: \.userId) { contact in
                    row(contact)
                        .listRowBackground(Color(colors.surfaceSubtle))
                        .listRowSeparatorTint(Color(colors.separator))
                }
            } header: {
                Text(title).foregroundStyle(Color(colors.textSecondary))
            }
        }
    }
}

private extension BotContactsView {
    private var normalizedUserId: String {
        newUserId.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// 加联系人：选对方是 Bot 还是人（核心不去对方那里查，ADR-0050），填完整的 Matrix ID。格式细节由核心判。
    private func addPanel(_ close: @escaping (@escaping () -> Void) -> Void) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            WontonGroup {
                StepOptionRow(title: String(localized: .contactsAddKindBot), isSelected: !addsPerson) {
                    addsPerson = false
                } trailing: {
                    EmptyView()
                }
                WontonSeparator()
                StepOptionRow(title: String(localized: .contactsAddKindPerson), isSelected: addsPerson) {
                    addsPerson = true
                } trailing: {
                    EmptyView()
                }
            }
            TextField(String(localized: .contactsAdd), text: $newUserId,
                      prompt: Text(.contactsAddPlaceholder))
                .keyboardType(.asciiCapable)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .submitLabel(.send)
                .focused($addFocused)
                .onSubmit { submitAdd(close) }
                .wontonField()
                .accessibilityIdentifier("contactsUserId")
            Button {
                submitAdd(close)
            } label: {
                Text(.contactsAddSubmit).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle())
            .accessibilityIdentifier("contactsSubmit")
            .disabled(normalizedUserId.isEmpty)
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.vertical, WontonTheme.spacing.s16)
        .task {
            newUserId = ""
            addsPerson = false
            addFocused = true
        }
    }

    /// 发出去就收起面板；结果和失败原因跟着核心的推送回到页面上。
    private func submitAdd(_ close: @escaping (@escaping () -> Void) -> Void) {
        let userId = normalizedUserId
        guard !userId.isEmpty else { return }
        model.send(addsPerson
            ? .requestPersonContact(bot: model.bot, userId: userId)
            : .requestBotContact(bot: model.bot, userId: userId))
        close {}
    }
}

/// 页面的状态：核心的 `BotContactsViewModel`，只取这个 Bot 的部分；加核心当场拒绝的操作。
@MainActor
@Observable
final class BotContactsModel {
    private(set) var viewModel: BotContactsViewModel
    /// 核心拒绝上一个操作的原因，由界面弹窗展示。
    var rejection: Problem?
    let bot: BotRef
    private let core: ClientCore

    init(core: ClientCore, bot: BotRef) {
        self.core = core
        self.bot = bot
        viewModel = core.botContacts()
    }

    /// ViewModel 一次只看一个 Bot：还看着别的 Bot 时（刚切过来）这里没有行，当成正在取。
    private var isCurrent: Bool { viewModel.bot == bot }

    var contacts: [BotContact] { isCurrent ? viewModel.contacts : [] }

    var isLoading: Bool {
        guard isCurrent else { return true }
        if case .loading = viewModel.state { return true }
        return false
    }

    var failure: (error: ContactError, message: String)? {
        guard isCurrent, case let .failed(error, message) = viewModel.state else { return nil }
        return (error, message)
    }

    /// 进页面时选定这个 Bot 取一次，之后跟着核心的推送走；页面消失时订阅结束。
    func observe() async {
        send(.refreshBotContacts(bot: bot))
        for await viewModel in core.subscribeBotContacts().updates() {
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

    func retry() {
        send(.refreshBotContacts(bot: bot))
    }

    /// 下拉刷新：等核心把状态从 Loading 换走，圈才收起来。
    func refresh() async {
        retry()
        guard rejection == nil else { return }
        for await viewModel in core.subscribeBotContacts().updates() {
            if viewModel.bot == bot, case .loading = viewModel.state { continue }
            return
        }
    }
}

extension View {
    /// 删除已生效联系人的确认框：挂在整页上，滑动删除和行内按钮只负责把它带出来。
    func contactsRemoveDialog(_ removing: Binding<BotContact?>, model: BotContactsModel) -> some View {
        let contact = removing.wrappedValue
        return wontonDialog(
            String(localized: .contactsRemoveTitle(contact.map(contactName) ?? "")),
            message: String(localized: .contactsRemoveMessage),
            isPresented: Binding(
                get: { removing.wrappedValue != nil }, set: { if !$0 { removing.wrappedValue = nil } }
            ),
            actions: [
                WontonDialogAction(title: String(localized: .commonCancel)) { removing.wrappedValue = nil },
                WontonDialogAction(title: String(localized: .commonRemove), kind: .danger) {
                    if let contact { model.send(.removeBotContact(bot: model.bot, userId: contact.userId)) }
                    removing.wrappedValue = nil
                }
            ]
        )
    }
}
