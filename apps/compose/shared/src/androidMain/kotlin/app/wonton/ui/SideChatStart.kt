package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardCapitalization
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import kotlinx.coroutines.launch

/** 输入框最多长到几行，再多就在框里滚。 */
private const val FIRST_MESSAGE_MAX_LINES = 6

/**
 * 开新旁聊的面板（ADR-0093 §6）：主人写第一句，发送时核心先发根消息、再把这句放进旁聊，成功后 [onStarted] 拿到根的
 * 事件 ID 进旁聊页。没开成时面板留着写好的字，按类别说原因，配重试（§7）。发送中关掉面板不撤回：根发出去了，
 * 旁聊照样出现在主对话里。
 */
@Composable
internal fun NewSideChatSheet(core: ClientCore, roomId: String, onStarted: (String) -> Unit, onDismiss: () -> Unit) {
    var body by rememberSaveable(roomId) { mutableStateOf("") }
    var sending by remember { mutableStateOf(false) }
    var failure by remember { mutableStateOf<Problem?>(null) }
    val scope = rememberCoroutineScope()
    val start = {
        sending = true
        failure = null
        scope.launch {
            try {
                onStarted(core.startSideChat(roomId, body))
            } catch (rejection: CoreException) {
                failure = rejection.problem
            } finally {
                sending = false
            }
        }
        Unit
    }
    WontonSheet(onDismiss = onDismiss, title = stringResource(R.string.side_chat_new)) {
        Column(
            Modifier.fillMaxWidth().padding(horizontal = WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            WontonTextField(
                value = body,
                onValueChange = { body = it },
                modifier = Modifier.fillMaxWidth(),
                placeholder = stringResource(R.string.side_chat_placeholder),
                maxLines = FIRST_MESSAGE_MAX_LINES,
                enabled = !sending,
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
            )
            failure?.let { ProblemRow(it, onRetry = start) }
            WontonButton(
                stringResource(R.string.composer_send),
                start,
                Modifier.fillMaxWidth(),
                enabled = body.isNotBlank() && !sending,
            )
        }
    }
}
