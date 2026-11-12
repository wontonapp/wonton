import SwiftUI
import WontonClient

struct InstallCompletionView: View {
    let model: PluginsModel
    let pluginId: String
    let close: () -> Void
    let review: () -> Void
    @Environment(OnboardingModel.self) private var onboarding
    @Environment(\.goToInstalledApps) private var goToInstalledApps

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s16) {
            if let task = model.viewModel.tasks.first(where: { $0.id == pluginId }) {
                TaskRow(
                    model: model,
                    task: task,
                    review: review,
                    checkInstalled: {
                        close()
                        goToInstalledApps()
                    },
                    showsDismiss: false,
                )
                if task.state == .succeeded {
                    if onboarding.viewModel.tips.appInstalled {
                        OnboardingTipView(text: String(localized: .Onboarding5.onboardingTipAppInstalled)) {
                            onboarding.send(.dismissOnboardingTip(tip: .appInstalled))
                        }
                    }
                    if let plugin = model.viewModel.installed?.plugins.first(where: { $0.id == pluginId }) {
                        PluginPinButton(plugin: plugin)
                    }
                    Button(TabText.done, action: close).buttonStyle(WontonButtonStyle())
                }
            } else {
                LoadingView()
            }
        }
        .padding(WontonTheme.spacing.pageInset)
    }
}
