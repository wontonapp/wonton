package app.wonton.ui

import android.icu.text.ListFormatter
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
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.OnboardingItemProblem
import app.wonton.protocol.OnboardingItemState
import app.wonton.protocol.OnboardingReview
import app.wonton.protocol.OnboardingReviewDetail
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
            Text(item.kind.section(), style = WontonTheme.typography.supporting,
                color = WontonTheme.colors.textSecondary.toColor())
            Text(
                ListFormatter.getInstance(LocalConfiguration.current.locales[0]).format(item.titles),
                style = WontonTheme.typography.itemTitle,
                color = WontonTheme.colors.textPrimary.toColor(),
            )
            item.detail?.let {
                Text(
                    it.line(),
                    style = WontonTheme.typography.supporting,
                    color = WontonTheme.colors.textSecondary.toColor(),
                )
            }
        }
    }
}

/** 每一行的类别，写在标题上面。 */
@Composable
private fun OnboardingReviewKind.section(): String = when (this) {
    OnboardingReviewKind.PROFILE -> stringResource(R.string.onboarding_review_section_profile)
    OnboardingReviewKind.AGENT -> stringResource(R.string.onboarding_review_section_agent)
    OnboardingReviewKind.MODEL -> stringResource(R.string.onboarding_review_section_model)
    OnboardingReviewKind.APP -> stringResource(R.string.onboarding_review_section_apps)
}

@Composable
private fun ReviewItemState(state: OnboardingItemState, retry: () -> Unit) {
    when (state) {
        is OnboardingItemState.Preparing -> ReviewProgress(stringResource(R.string.onboarding_review_preparing))
        is OnboardingItemState.Ready -> Unit
        is OnboardingItemState.Blocked -> Text(
            state.message ?: stringResource(R.string.onboarding_review_blocked),
            style = WontonTheme.typography.body,
            color = WontonTheme.colors.danger.toColor(),
        )
        is OnboardingItemState.Waiting -> Text(
            stringResource(R.string.onboarding_review_waiting),
            style = WontonTheme.typography.supporting, color = WontonTheme.colors.textSecondary.toColor(),
        )
        is OnboardingItemState.Running -> ReviewProgress(stringResource(R.string.onboarding_running))
        is OnboardingItemState.Done -> Icon(
            WontonIcon.Check, contentDescription = stringResource(R.string.onboarding_review_done),
            modifier = Modifier.size(WontonTheme.size.icon),
            tint = WontonTheme.colors.success.toColor(),
        )
        is OnboardingItemState.Uncertain -> key(state.message) {
            OnboardingProblem(stringResource(R.string.onboarding_review_uncertain), state.message)
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

/** 第二行的文字，按类别写在这里。 */
@Composable
private fun OnboardingReviewDetail.line(): String = when (this) {
    is OnboardingReviewDetail.Email -> email
    is OnboardingReviewDetail.SetAsDefault -> stringResource(R.string.onboarding_review_set_default)
    is OnboardingReviewDetail.KeepCurrent -> stringResource(R.string.onboarding_review_keep_current)
    is OnboardingReviewDetail.Model -> model
    is OnboardingReviewDetail.Unreachable -> model
        ?.let { stringResource(R.string.onboarding_review_unreachable_model, it) }
        ?: stringResource(R.string.onboarding_review_unreachable)
    is OnboardingReviewDetail.SignInAfterSetup -> stringResource(R.string.onboarding_review_sign_in_after_setup)
    is OnboardingReviewDetail.Unchanged -> stringResource(R.string.onboarding_review_unchanged)
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
