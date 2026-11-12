import SwiftUI
import WontonClient

struct OnboardingAppsStep: View {
    let needs: [OnboardingNeedRow]
    let catalogProblem: StoreProblem?
    let plugins: PluginsModel
    let send: (Action) -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            if let catalogProblem {
                ProblemNotice(problem: catalogProblem.onboardingProblem)
            }
            ForEach(needs, id: \.id) { need in
                StepOptionRow(title: need.title, isSelected: need.selected) {
                    send(.toggleOnboardingNeed(id: need.id))
                }
                // 子项跟着分组一起勾：勾的粒度是需求，子项列出它装哪些应用，点子项同切换。
                ForEach(need.plugins, id: \.id) { item in
                    StepOptionRow(
                        title: item.title, isSelected: need.selected, blocked: item.blocked,
                        select: { send(.toggleOnboardingNeed(id: need.id)) },
                        leading: {
                            PluginIconView(icon: item.icon, size: WontonTheme.size.taskIcon, model: plugins)
                        },
                        trailing: {
                            if item.installedVersion != nil {
                                Text(.onboardingAppInstalled)
                                    .wontonText(WontonTheme.typography.supporting)
                                    .foregroundStyle(Color(colors.success))
                            }
                        }
                    )
                    .padding(.leading, WontonTheme.spacing.s16)
                }
            }
        }
    }
}
