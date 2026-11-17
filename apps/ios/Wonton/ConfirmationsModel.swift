import Observation
import WontonClient

/// “等你确认”列表的模型：订阅核心的列表，壳只读；问题不弹窗，列表页照实显示并给重试。
@MainActor
@Observable
final class ConfirmationsModel {
    private(set) var viewModel: ConfirmationsViewModel
    private(set) var contacts: BotContactsViewModel?
    private(set) var actingBot: BotRef?

    var pendingCount: Int { viewModel.pending.count + viewModel.contactRequests.count }

    var contactFailure: Problem? {
        guard let contacts, contacts.bot == actingBot else { return nil }
        if let failure = contacts.actionFailure {
            return contactsProblem(failure.error, message: failure.message)
        }
        if case let .failed(error, message) = contacts.state {
            return contactsProblem(error, message: message)
        }
        return nil
    }

    var contactBusy: Bool {
        guard let actingBot else { return false }
        guard let contacts, contacts.bot == actingBot else { return true }
        return contacts.state == .loading
    }

    func observeContacts() async {
        for await _ in core.subscribeBotContacts().updates() { contacts = core.botContacts() }
    }

    func act(on request: PendingContactRequest, approve: Bool, chat: ChatModel) {
        guard !contactBusy else { return }
        actingBot = request.bot
        do {
            try core.dispatch(action: .botContacts(action: approve
                ? .approveBotContact(bot: request.bot, userId: request.contact.userId)
                : .rejectBotContact(bot: request.bot, userId: request.contact.userId)))
            contacts = core.botContacts()
        } catch {
            actingBot = nil
            chat.rejection = error.problem
        }
    }

    let core: ClientCore

    init(core: ClientCore) {
        self.core = core
        viewModel = core.confirmations()
    }

    /// 在视图的 `.task` 里调用；视图消失时 Task 被取消，订阅随之结束。
    func observe() async {
        for await _ in core.subscribeConfirmations().updates() {
            viewModel = core.confirmations()
        }
    }
}
