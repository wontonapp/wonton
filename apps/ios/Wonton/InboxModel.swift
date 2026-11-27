import Observation
import UserNotifications
import WontonClient

/// 收件箱的模型（ADR-0119）：订阅核心合好的收件箱，壳只读；确认卡片是其中一类条目。问题不弹窗，页面照实显示并给重试。
@MainActor
@Observable
final class InboxModel {
    private(set) var viewModel: InboxViewModel
    private(set) var contacts: BotContactsViewModel?
    private(set) var actingBot: BotRef?

    /// 铃铛上的数：等你回答的条数。还没取到、或最近一次没取成时不确定，是 `nil`，不当成 0。
    var needing: UInt32? {
        viewModel.loading || viewModel.problem != nil ? nil : viewModel.list.counts.needing
    }

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
        viewModel = core.inbox()
    }

    /// 在主界面和收件箱页的 `.task` 里调用（收件箱页可能在单独的窗口里）。
    /// 视图消失时 Task 被取消，订阅随之结束；回来时先补上错过的那份。
    func observe() async {
        viewModel = core.inbox()
        retractEndedNotifications()
        for await _ in core.subscribeInbox().updates() {
            viewModel = core.inbox()
            retractEndedNotifications()
        }
    }

    /// 已经不用回答的确认卡片，通知中心里的那条也撤掉（Android 按 `settled_confirmations` 撤，见
    /// `PushNotifications.cancelCards`）：卡片在别的设备上答了、过期了、被撤回，或者核心已经不认识它。
    /// 收件箱确实取到了才撤，没取到时不知道哪些还在等，不动。收件箱只是当前这台服务器的，别的服务器的通知不碰。
    private func retractEndedNotifications() {
        guard !viewModel.loading, viewModel.problem == nil,
            let server = core.servers().servers.first(where: \.current)?.id
        else { return }
        var waiting = Set<String>()
        for entry in viewModel.entries where entry.item.kind == .confirm && entry.item.status == .pending {
            // 条目到了、卡片还没到这台设备：这一轮不知道它的卡片 ID，先都不撤，等下一份。
            guard let card = entry.card else { return }
            waiting.insert(card.card.id)
        }
        Task {
            let center = UNUserNotificationCenter.current()
            let ended = await center.deliveredNotifications().compactMap { notification -> String? in
                let info = notification.request.content.userInfo
                guard info["server"] as? String == server,
                    let card = info[PushUserInfo.confirmationId] as? String,
                    !waiting.contains(card)
                else { return nil }
                return notification.request.identifier
            }
            if !ended.isEmpty { center.removeDeliveredNotifications(withIdentifiers: ended) }
        }
    }
}

/// 通知扩展和 App 之间约定的 `userInfo` 键。
enum PushUserInfo {
    /// 确认卡片推送的卡片 ID（`WontonPushExtension/NotificationService.swift` 写）。
    static let confirmationId = "confirmation_id"
}
