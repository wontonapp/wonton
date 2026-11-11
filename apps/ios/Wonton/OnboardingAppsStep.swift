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
                if need.selected {
                    ForEach(need.plugins, id: \.id) { item in
                        plugin(item)
                    }
                }
            }
        }
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
            if item.installedVersion != nil {
                Text(.Onboarding4.onboardingAppInstalled)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.success))
            }
        }
        .padding(.leading, WontonTheme.spacing.s16)
        .padding(.vertical, WontonTheme.spacing.s8)
    }
}
