import SwiftUI
import WontonClient

struct OnboardingAppsStep: View {
    let needs: [OnboardingNeedRow]
    let agents: [OnboardingAgentRow]
    let catalogProblem: StoreProblem?
    let plugins: PluginsModel
    let send: (Action) -> Void
    let install: (PluginConfirmation) -> Void
    @State private var reviewProblem: Problem?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            if let catalogProblem {
                ProblemNotice(problem: catalogProblem.onboardingProblem)
            }
            if let reviewProblem { ProblemNotice(problem: reviewProblem) }
            ForEach(plugins.viewModel.tasks, id: \.id) { task in
                TaskRow(model: plugins, task: task) {
                    if let confirmation = plugins.reconfirmation(for: task, source: source(for: task.id)) {
                        reviewProblem = nil
                        install(confirmation)
                    } else {
                        reviewProblem = Problem(title: "找不到这个应用的安装来源", nextStep: "刷新引导后再试。")
                    }
                }
            }
            ForEach(needs, id: \.id) { need in
                StepOptionRow(title: need.title, isSelected: need.selected) {
                    send(.toggleOnboardingNeed(id: need.id))
                }
                if need.selected {
                    ForEach(need.plugins, id: \.id) { item in
                        plugin(item)
                    }
                }
            }
        }
    }

    private func source(for id: String) -> String? {
        needs.lazy.flatMap(\.plugins).first { $0.id == id }?.source
            ?? agents.first { $0.id == id }?.source
    }

    private func plugin(_ item: StoreItem) -> some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            PluginIconView(icon: item.icon, size: WontonTheme.size.taskIcon, model: plugins)
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(verbatim: item.title)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                if let blocked = item.blocked {
                    Text(verbatim: blocked)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.danger))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if plugins.isWorking(on: item.id) {
                ProgressView().accessibilityLabel("正在安装")
            } else if item.installedVersion != nil {
                Text("已安装")
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.success))
            } else {
                Button("安装") { install(.install(source: item.source)) }
                    .buttonStyle(WontonSmallButtonStyle())
                    .disabled(item.blocked != nil)
            }
        }
        .padding(.leading, WontonTheme.spacing.s16)
        .padding(.vertical, WontonTheme.spacing.s8)
    }
}
