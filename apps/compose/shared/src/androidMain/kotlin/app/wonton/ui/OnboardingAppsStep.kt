package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource

import app.wonton.protocol.Action
import app.wonton.protocol.OnboardingAction
import app.wonton.protocol.OnboardingViewModel
import app.wonton.protocol.StoreItem
import app.wonton.ui.design.StepOptionRow
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

@Composable
internal fun OnboardingAppsStep(
    view: OnboardingViewModel,
    plugins: PluginsState,
    send: (Action) -> Unit,
) {
    view.catalogProblem?.let {
        OnboardingProblem(it.onboardingTitle(), it.details) {
            send(Action.Onboarding(OnboardingAction.RefreshOnboarding))
        }
    }
    view.needs.forEach { need ->
        key(need.id) {
            StepOptionRow(
                title = need.title,
                selected = need.selected,
                onSelect = { send(Action.Onboarding(OnboardingAction.ToggleOnboardingNeed(need.id))) },
                multiple = true,
            )
            // 子项跟着分组一起勾：勾的粒度是需求，子项列出它装哪些应用，点子项同切换。
            Column(
                Modifier.padding(start = WontonTheme.spacing.s16),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                need.plugins.forEach { item ->
                    key(item.id) {
                        OnboardingAppRow(item, need.selected, plugins) {
                            send(Action.Onboarding(OnboardingAction.ToggleOnboardingNeed(need.id)))
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun OnboardingAppRow(item: StoreItem, selected: Boolean, plugins: PluginsState, onToggle: () -> Unit) {
    StepOptionRow(
        title = item.title,
        selected = selected,
        onSelect = onToggle,
        blocked = item.blocked,
        leading = { PluginIconImage(item.icon, WontonTheme.size.taskIcon, plugins) },
        trailing = {
            if (item.installedVersion != null) {
                Text(
                    stringResource(R.string.onboarding_app_installed),
                    Modifier.padding(WontonTheme.spacing.s12),
                    style = WontonTheme.typography.supporting,
                    color = WontonTheme.colors.textSecondary.toColor(),
                )
            }
        },
    )
}
