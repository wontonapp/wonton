package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource

import app.wonton.protocol.OnboardingAction
import app.wonton.protocol.Action
import app.wonton.protocol.DefaultBotVisibility
import app.wonton.protocol.OnboardingTip
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.WontonIcon

@Composable
internal fun OnboardingChatPrompts(roomId: String) {
    val context = LocalOnboarding.current
    val view = context.view
    val room = context.chat.rooms.firstOrNull { it.roomId == roomId }
    val draft = LocalTabDraft.current
    if (view.defaultBotVisibility != DefaultBotVisibility.VISIBLE ||
        view.defaultBotUserId == null || room?.directUserId != view.defaultBotUserId
    ) return
    Column(
        Modifier.fillMaxWidth().padding(horizontal = WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        view.chatPrompts.forEach { prompt ->
            WontonButton(prompt, { draft.text = prompt }, kind = WontonButtonKind.SECONDARY)
        }
    }
}

@Composable
internal fun OnboardingTipNotice(tip: OnboardingTip) {
    val context = LocalOnboarding.current
    val shown = when (tip) {
        OnboardingTip.CONFIRMATION_CARD -> context.view.tips.confirmationCard
        OnboardingTip.APP_INSTALLED -> context.view.tips.appInstalled
    }
    if (!shown) return
    val text = stringResource(
        when (tip) {
            OnboardingTip.CONFIRMATION_CARD -> R.string.onboarding_tip_confirmation_card
            OnboardingTip.APP_INSTALLED -> R.string.onboarding_tip_app_installed
        },
    )
    Row(
        Modifier.fillMaxWidth().padding(horizontal = WontonTheme.spacing.pageInset),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Text(
            text,
            Modifier.weight(1f),
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        WontonIconButton(WontonIcon.Close, stringResource(R.string.common_close), {
            context.send(Action.Onboarding(OnboardingAction.DismissOnboardingTip(tip)))
        })
    }
}
