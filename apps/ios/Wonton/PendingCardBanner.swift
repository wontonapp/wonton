import SwiftUI
import WontonClient

/// 有卡片等主人回答时顶在页面最上面，点了去卡片所在的房间。多张时去最早到期的那张。
struct PendingCardBanner: View {
    let cards: [PendingConfirmation]
    let open: (String?) -> Void

    @Environment(\.wontonColors) private var colors
    @Environment(OnboardingModel.self) private var onboarding

    var body: some View {
        Button {
            open(cards[0].roomId)
        } label: {
            HStack(spacing: WontonTheme.spacing.s8) {
                WontonIcon.notify.view(size: WontonTheme.size.iconSmall)
                Text(title)
                    .wontonText(WontonTheme.typography.supporting)
                    .lineLimit(2)
                    .frame(maxWidth: .infinity, alignment: .leading)
                WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
            }
            .foregroundStyle(Color(colors.textOnAccentFill))
            .padding(.horizontal, WontonTheme.spacing.s16)
            .padding(.vertical, WontonTheme.spacing.s12)
            .frame(minHeight: WontonTheme.size.target)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("pending-confirmations-banner")
        .background(alignment: .bottom) {
            if let border = colors.accentFillBorder {
                Color(border).frame(height: WontonTheme.size.hairline)
            }
        }
        .background(Color(colors.accentFill), ignoresSafeAreaEdges: .top)
        if onboarding.viewModel.tips.confirmationCard {
            OnboardingTipView(text: String(localized: .Onboarding1.onboardingTipConfirmationCard)) {
                onboarding.send(.onboarding(action: .dismissOnboardingTip(tip: .confirmationCard)))
            }
        }
    }

    /// 只有一张卡片时说是什么事。
    private var title: String {
        if cards.count == 1 {
            return String(localized: .Onboarding1.onboardingPendingCardOne(cards[0].title))
        }
        return String(localized: .Onboarding0.onboardingPendingCardMany(cards: cards.count))
    }
}
