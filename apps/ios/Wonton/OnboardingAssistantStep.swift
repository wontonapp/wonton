import SwiftUI
import WontonClient

/// 你的助手这一步：给默认 Bot 起名字（ADR-0129 第 5 节）。只记进引导草稿，点完成时才保存。
struct OnboardingAssistantStepView: View {
    let assistant: OnboardingAssistantStep
    let send: (Action) -> Void
    @State private var name: String
    @Environment(\.wontonColors) private var colors

    init(assistant: OnboardingAssistantStep, send: @escaping (Action) -> Void) {
        self.assistant = assistant
        self.send = send
        _name = State(initialValue: assistant.name)
    }

    var body: some View {
        DetailSection(title: String(localized: .Onboarding3.onboardingAssistantName)) {
            TextField("", text: Binding(get: { name }, set: {
                name = $0
                send(.onboarding(action: .editOnboardingAssistant(name: name)))
            }))
            .accessibilityLabel(String(localized: .Onboarding3.onboardingAssistantName))
            .textFieldStyle(.plain)
            .wontonField()
            if let problem = assistant.nameProblem {
                Text(verbatim: problem.problem)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.danger))
            }
        }
    }
}
