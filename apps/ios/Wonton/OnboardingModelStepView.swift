import SwiftUI
import WontonClient

/// 接模型这一步就是默认 Bot 的模型页（ADR-0129 第 4 节）：页里的操作当场存进 Bot，不进引导草稿；
/// 下一步能不能点由核心的 `StepNav` 说了算。
struct OnboardingModelStepView: View {
    let step: OnboardingModelStep
    @State private var models: AgentModelsModel

    init(step: OnboardingModelStep, core: ClientCore) {
        self.step = step
        _models = State(initialValue: AgentModelsModel(core: core))
    }

    var body: some View {
        AgentModelsPage(view: models.viewModel, rejection: models.rejection, send: models.send)
            .task { await models.observe() }
            .task(id: step.bot) {
                if let bot = step.bot { models.send(.openAgentModels(bot: bot)) }
            }
    }
}
