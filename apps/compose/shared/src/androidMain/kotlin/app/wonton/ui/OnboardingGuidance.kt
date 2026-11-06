package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier

import app.wonton.protocol.Action
import app.wonton.protocol.OnboardingTip
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
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
    if (view.primaryBotUserId == null || room?.directUserId != view.primaryBotUserId) return
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
    val text = when (tip) {
        OnboardingTip.CONFIRMATION_CARD -> "需要确认时，在聊天里的卡片上点确认。"
        OnboardingTip.APP_INSTALLED -> "应用装好后，可以把它放到底栏。"
    }
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
        WontonIconButton(WontonIcon.Close, "关闭", {
            context.send(Action.DismissOnboardingTip(tip))
        })
    }
}

@Composable
internal fun ReplacedAgentOffer(plugins: PluginsState) {
    val context = LocalOnboarding.current
    val offer = context.view.replacedAgent
    var answered by remember(offer?.id) { mutableStateOf(false) }
    var uninstall by remember { mutableStateOf<UninstallTarget?>(null) }
    if (offer != null && !answered) {
        val dismiss = {
            answered = true
            context.send(Action.DismissReplacedAgentOffer)
        }
        WontonDialog(
            "要卸掉“${offer.title}”吗？",
            onDismiss = dismiss,
            actions = listOf(
                WontonDialogAction("卸载") {
                    uninstall = UninstallTarget(offer.id, offer.title)
                    dismiss()
                },
                WontonDialogAction("保留", onClick = dismiss),
            ),
        )
    }
    uninstall?.let { target -> UninstallDialog(target, plugins) { uninstall = null } }
}
