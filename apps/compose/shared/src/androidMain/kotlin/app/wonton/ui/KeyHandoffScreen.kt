package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.ChatViewModel
import app.wonton.protocol.KeyHandoff
import app.wonton.protocol.TabRef
import app.wonton.protocol.VerificationStage
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Error
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

/**
 * 加设备时等另一台设备交出聊天密钥（ADR-0087）：整屏盖住聊天列表，代替“输入恢复密钥”。该不该显示、走到哪一步
 * 都来自核心（`EncryptionStatus.keyHandoff` 和 `verificationFlow`）；主人选“稍后”只在这台 App 里先收起
 * （[ChatNavigation.keyHandoffDeferred]），聊天列表顶上的“在另一台设备上继续”再打开。
 */
@Composable
internal fun KeyHandoffHost(chat: ChatViewModel, navigation: AppNavigation, core: ClientCore) {
    if (chat.encryption.keyHandoff !is KeyHandoff.Receiving || navigation.chat.keyHandoffDeferred) return
    Dialog(
        onDismissRequest = {},
        properties = DialogProperties(
            usePlatformDefaultWidth = false,
            dismissOnBackPress = false,
            dismissOnClickOutside = false,
            decorFitsSystemWindows = false,
        ),
    ) {
        KeyHandoffScreen(
            chat,
            core,
            onLater = { navigation.chat.keyHandoffDeferred = true },
            onNoOtherDevice = {
                navigation.pages.send(Action.OpenPage(TabRef.Chat))
                navigation.chat.showingBackup = true
            },
        )
    }
}

@Composable
internal fun KeyHandoffScreen(
    chat: ChatViewModel,
    core: ClientCore,
    onLater: () -> Unit,
    onNoOtherDevice: () -> Unit,
) {
    val colors = WontonTheme.colors
    // 核心当场拒绝的原因，比如这时还发不了新请求。
    var refusal by remember { mutableStateOf<Problem?>(null) }
    val onAction: (Action) -> Unit = { action -> refusal = core.send(action) }
    Box(Modifier.fillMaxSize().background(colors.canvas.toColor())) {
        Column(
            Modifier.fillMaxSize().safeDrawingPadding().verticalScroll(rememberScrollState())
                .padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            Text(
                stringResource(R.string.handoff_title),
                style = WontonTheme.typography.screenTitle,
                color = colors.textPrimary.toColor(),
            )
            refusal?.let { ProblemNotice(it, WontonIcon.Error) }
            HandoffProgress(chat, onAction, onLater)
            Text(
                stringResource(R.string.handoff_no_other_device),
                Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
                    .clickable {
                        refusal = core.send(Action.SkipKeyHandoff)
                        if (refusal == null) onNoOtherDevice()
                    }
                    .padding(vertical = WontonTheme.spacing.s12),
                style = WontonTheme.typography.supporting,
                color = colors.textSecondary.toColor(),
                textAlign = TextAlign.Center,
            )
        }
    }
}

@Composable
private fun HandoffProgress(chat: ChatViewModel, onAction: (Action) -> Unit, onLater: () -> Unit) {
    val flow = chat.encryption.verificationFlow?.takeIf { it.startedHere }
    val handoff = chat.encryption.keyHandoff
    when {
        handoff is KeyHandoff.Receiving && handoff.unanswered -> WontonNotice(
            WontonIcon.Warning,
            stringResource(R.string.handoff_unanswered_title),
            lines = listOf(stringResource(R.string.handoff_unanswered_message)),
        ) {
            Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                WontonButton(
                    stringResource(R.string.handoff_try_again),
                    { onAction(Action.StartDeviceVerification) },
                    Modifier.weight(1f),
                )
                WontonButton(
                    stringResource(R.string.handoff_later),
                    onLater,
                    Modifier.weight(1f),
                    WontonButtonKind.SECONDARY,
                )
            }
        }
        // 对方接上以后和聊天备份页里的确认一样：对表情、等对方、取密钥，没成时给“重新开始”。
        flow != null && flow.stage != VerificationStage.Requested ->
            VerificationFlowContent(flow, onAction, onUseRecoveryKey = null)
        // 请求发出还没人接：这时没有要主人在这台设备上做的，只说去另一台设备上。
        else -> Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            CircularProgressIndicator(
                Modifier.size(WontonTheme.size.iconSmall),
                color = WontonTheme.colors.textPrimary.toColor(),
                strokeWidth = 2.dp,
            )
            Text(
                stringResource(R.string.handoff_waiting),
                style = WontonTheme.typography.body,
                color = WontonTheme.colors.textPrimary.toColor(),
            )
        }
    }
}
