#if DEBUG
import SwiftUI
import WontonClient

/// 界面走查的夹具屏：不走引导，直接摆助手选择行。启动参数带
/// `wonton-onboarding-agents-preview` 时生效（同 `QualityPreviews` 的做法）。
/// 假数据照官方清单的英文标题（界面文字检查不豁免 App 目标里的中文假数据，
/// 先例 AgentRunPreview 同为字面量夹具）：一个选中、一个带订阅登录列表、一个装不了。
enum OnboardingAgentsPreview {
    static var active: Bool {
        ProcessInfo.processInfo.arguments.contains("wonton-onboarding-agents-preview")
    }

    static var root: some View {
        OnboardingAgentStep(agents: fixtures()) { _ in }
            .padding(WontonTheme.spacing.s16)
    }

    private static func fixtures() -> [OnboardingAgentRow] {
        [
            row("hermes", "Hermes", [], true, nil),
            row(
                "omp", "Coding Bot (omp)",
                [
                    "ChatGPT subscription sign-in", "GitHub Copilot subscription sign-in",
                    "magpie model gateway",
                ],
                false, nil
            ),
            row("openclaw", "All-Purpose Bot (OpenClaw)", [], false, "Update the core first"),
        ]
    }

    /// 文字检查的规则二：夹具的字面量不经 label: 参数位（同 AgentRunPreview.advance）。
    private static func row(
        _ id: String, _ title: String, _ logins: [String], _ selected: Bool, _ blocked: String?
    ) -> OnboardingAgentRow {
        OnboardingAgentRow(id: id, title: title, logins: logins, selected: selected, source: nil, blocked: blocked)
    }
}
#endif
