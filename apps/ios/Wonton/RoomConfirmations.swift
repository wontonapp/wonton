import SwiftUI
import WontonClient

/// The room subscription supplies these records independently of Matrix messages.
struct RoomConfirmationBanner: View {
    let cards: [ConfirmationView]
    let open: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        if let first = cards.first {
            Button(action: open) {
                HStack(spacing: WontonTheme.spacing.s8) {
                    WontonIcon.notify.view(size: WontonTheme.size.iconSmall)
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                        Text(.ConfirmationCard.confirmationCardRequests)
                            .wontonText(WontonTheme.typography.caption)
                        Text(first.card.title)
                            .wontonText(WontonTheme.typography.supporting)
                            .lineLimit(1)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                }
                .foregroundStyle(Color(colors.textOnAccentFill))
                .padding(.horizontal, WontonTheme.spacing.pageInset)
                .padding(.vertical, WontonTheme.spacing.s8)
                .frame(minHeight: WontonTheme.size.target)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("room-confirmations")
            .background(Color(colors.accentFill))
        }
    }
}

struct RoomConfirmationSheet: View {
    let cards: [ConfirmationView]
    let selectedId: String?
    let action: (Action) -> Void

    var body: some View {
        ScrollViewReader { scroller in
            ScrollView {
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
                    if cards.isEmpty {
                        Text(.ConfirmationCard.confirmationCardUnavailable)
                            .wontonText(WontonTheme.typography.supporting)
                    }
                    ForEach(cards, id: \.card.id) { view in
                        ConfirmationCardView(
                            view: view,
                            answer: { choice, secrets, settings, devices in
                                action(.answerConfirmation(
                                    id: view.card.id, answer: choice.answer, choiceId: choice.choiceId,
                                    secrets: secrets, settings: settings, devices: devices
                                ))
                            },
                            stopWindow: { action(.stopConfirmationWindow(id: view.card.id)) }
                        )
                        .accessibilityElement(children: .contain)
                        .accessibilityIdentifier("confirmation-expanded-\(view.card.id)")
                        .id(view.card.id)
                        if view.card.id != cards.last?.card.id { Divider() }
                    }
                }
                .padding(WontonTheme.spacing.pageInset)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .onAppear { scrollToSelection(scroller) }
            .onChange(of: selectedId) { scrollToSelection(scroller) }
            .onChange(of: cards.map(\.card.id)) { scrollToSelection(scroller) }
        }
    }

    private func scrollToSelection(_ scroller: ScrollViewProxy) {
        guard let selectedId else { return }
        scroller.scrollTo(selectedId, anchor: .top)
    }
}
