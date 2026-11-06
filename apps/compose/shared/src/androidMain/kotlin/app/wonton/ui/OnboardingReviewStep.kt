package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import app.wonton.protocol.Action
import app.wonton.protocol.OnboardingItemProblem
import app.wonton.protocol.OnboardingItemState
import app.wonton.protocol.OnboardingReview
import app.wonton.protocol.OnboardingReviewItem
import app.wonton.protocol.OnboardingReviewKind
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.WontonIcon

@Composable
internal fun OnboardingReviewStep(review: OnboardingReview, plugins: PluginsState, send: (Action) -> Unit) {
    review.items.forEach { item ->
        key(item.id) {
            WontonGroup {
                Column(
                    Modifier.fillMaxWidth().padding(WontonTheme.spacing.s16),
                    verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
                ) {
                    ReviewItemHeading(item, plugins)
                    InstallAccessDetails(item.notices, item.permissions)
                    ReviewItemState(item.state) { send(Action.RetryOnboardingItem(item.id)) }
                }
            }
        }
    }
}

@Composable
private fun ReviewItemHeading(item: OnboardingReviewItem, plugins: PluginsState) {
    Row(
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (item.kind == OnboardingReviewKind.AGENT || item.kind == OnboardingReviewKind.APP) {
            PluginIconImage(item.icon, WontonTheme.size.taskIcon, plugins)
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(item.title, style = WontonTheme.typography.itemTitle, color = WontonTheme.colors.textPrimary.toColor())
            item.detail?.takeIf { it.isNotEmpty() }?.let {
                Text(it, style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor())
            }
        }
    }
}

@Composable
private fun ReviewItemState(state: OnboardingItemState, retry: () -> Unit) {
    when (state) {
        is OnboardingItemState.Preparing -> ReviewProgress("正在准备")
        is OnboardingItemState.Ready -> Unit
        is OnboardingItemState.Blocked -> Text(
            state.message, style = WontonTheme.typography.body, color = WontonTheme.colors.danger.toColor(),
        )
        is OnboardingItemState.Waiting -> Text(
            "排队", style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor(),
        )
        is OnboardingItemState.Running -> ReviewProgress("进行中")
        is OnboardingItemState.Done -> Icon(
            WontonIcon.Check, contentDescription = "完成", modifier = Modifier.size(WontonTheme.size.icon),
            tint = WontonTheme.colors.success.toColor(),
        )
        is OnboardingItemState.Uncertain -> key(state.message) {
            OnboardingProblem("不确定装好没有，请到应用页查看。", state.message)
        }
        is OnboardingItemState.Failed -> key(state.problem) {
            when (val problem = state.problem) {
                is OnboardingItemProblem.Plugin -> OnboardingProblem(
                    problem.problem.onboardingTitle(), problem.problem.message, retry,
                )
                is OnboardingItemProblem.Model -> OnboardingProblem(
                    problem.problem.onboardingTitle(), problem.problem.message, retry,
                )
            }
        }
    }
}

@Composable
private fun ReviewProgress(label: String) {
    Row(
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CircularProgressIndicator(
            modifier = Modifier.size(WontonTheme.size.iconSmall),
            color = WontonTheme.colors.textPrimary.toColor(),
            strokeWidth = WontonTheme.size.hairline,
        )
        Text(label, style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor())
    }
}
