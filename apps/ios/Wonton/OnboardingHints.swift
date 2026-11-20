import SwiftUI
import WontonClient

struct OnboardingTipView: View {
    let text: String
    let dismiss: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s8) {
            Text(verbatim: text)
                .wontonText(WontonTheme.typography.supporting)
                .foregroundStyle(Color(colors.textSecondary))
                .frame(maxWidth: .infinity, alignment: .leading)
            WontonIconButton(icon: .close, label: String(localized: .Common.commonClose), action: dismiss)
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
    }
}

struct OnboardingChatPrompts: View {
    let prompts: [String]
    let draft: (String) -> Void

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: WontonTheme.spacing.s8) {
                ForEach(Array(prompts.enumerated()), id: \.offset) { _, prompt in
                    Button { draft(prompt) } label: { Text(verbatim: prompt) }
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            }
            .padding(.horizontal, WontonTheme.spacing.pageInset)
        }
        // 界面测试按它让开这一排：从这里起手的上推归这排自己的横滚，不拖消息列表。
        .accessibilityIdentifier("onboardingPrompts")
    }
}
