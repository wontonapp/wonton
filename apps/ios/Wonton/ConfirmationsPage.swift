import SwiftUI
import WontonClient

struct ConfirmationsPage: View {
    let model: ConfirmationsModel
    let chat: ChatModel
    let plugins: PluginsModel
    @State private var approvingRemote: PendingContactRequest?

    var body: some View {
        ConfirmationsListView(
            view: model.viewModel, bots: chat.viewModel.bots, plugins: plugins,
            answer: { card, choice, secrets, settings in
                chat.send(.answerConfirmation(id: card.card.id, answer: choice.answer,
                    choiceId: choice.choiceId, secrets: secrets, settings: settings))
            },
            stopWindow: { chat.send(.stopConfirmationWindow(id: $0.card.id)) },
            retry: { chat.send(.retryConfirmations) },
            approveContact: { request in
                if request.contact.remote == nil { model.act(on: request, approve: true, chat: chat) }
                else { approvingRemote = request }
            },
            rejectContact: { model.act(on: $0, approve: false, chat: chat) },
            contactFailure: model.contactFailure, contactBusy: model.contactBusy
        )
        .task { await model.observe() }
        .task { await model.observeContacts() }
        .wontonDialog(
            String(localized: .Contacts1.contactsApproveRemoteTitle(approvingRemote.map { contactName($0.contact) } ?? "")),
            message: String(localized: .Contacts1.contactsApproveRemoteMessage),
            isPresented: Binding(get: { approvingRemote != nil }, set: { if !$0 { approvingRemote = nil } }),
            actions: [
                WontonDialogAction(title: String(localized: .Common.commonCancel)) { approvingRemote = nil },
                WontonDialogAction(title: String(localized: .Contacts1.contactsApprove), kind: .primary) {
                    if let request = approvingRemote { model.act(on: request, approve: true, chat: chat) }
                    approvingRemote = nil
                }
            ]
        )
    }
}
