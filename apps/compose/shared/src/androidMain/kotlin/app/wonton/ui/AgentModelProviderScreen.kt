package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import app.wonton.protocol.AgentModelsAction
import app.wonton.protocol.ModelProviderPage
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.WontonIcon

/** 点开的一家服务商：登录、填 key、选模型、断开，按核心给的接法摆（ADR-0129 第 3 节）。 */
@Composable
internal fun AgentModelProviderScreen(page: ModelProviderPage, models: AgentModels) {
    val send = models.send
    val close = { send(AgentModelsAction.CloseModelProvider) }
    // 在这一页选了模型：核心换上以后收回这一页，测试结果就在顶上（和 iOS 一样）。
    val current = models.view.current
    var shown by remember { mutableStateOf(current?.provider to current?.model) }
    LaunchedEffect(current) {
        val next = current?.provider to current?.model
        if (next != shown) {
            shown = next
            if (current?.provider == page.row.id) close()
        }
    }
    BackHandler(onBack = close)
    WontonScreen(bar = {
        WontonPushedBar(page.row.name, close, actions = { if (page.busy) BusyMark() })
    }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            val problem = models.view.actionProblem
            problem?.takeIf { !it.isFieldProblem() }?.let { key(it) { ProblemRow(it.problem) } }
            if (page.row.connected) ConnectedLine(page.row.account)
            // 一家一份输入：换一家时填到一半的 key 不带过去。
            key(page.row.id) {
                // 接好以后先选模型；重新登录、换 key 排到后面。
                if (page.row.connected) ModelListSection(page, models.view.current, send)
                ModelLoginSection(page, send, models.browser)
                page.key?.takeIf { page.login == null }?.let { form ->
                    ModelKeySection(page, form, problem?.takeIf { it.isFieldProblem() }, send, models.browser)
                }
                if (!page.row.connected) ModelListSection(page, models.view.current, send)
            }
            if (page.row.connected && page.login == null) DisconnectButton(page, send)
        }
    }
    page.confirm?.let { confirm ->
        WontonDialog(
            title = stringResource(R.string.agent_models_confirm_title, confirm.model),
            message = confirm.message,
            onDismiss = { send(AgentModelsAction.DismissModelConfirmation) },
            actions = listOf(
                WontonDialogAction(stringResource(R.string.common_cancel)) {
                    send(AgentModelsAction.DismissModelConfirmation)
                },
                WontonDialogAction(stringResource(R.string.agent_models_use), WontonButtonKind.PRIMARY) {
                    send(AgentModelsAction.ConfirmModelSelection)
                },
            ),
        )
    }
}

@Composable
private fun BusyMark() {
    val label = stringResource(R.string.design_busy)
    Box(Modifier.size(WontonTheme.size.target), contentAlignment = Alignment.Center) {
        CircularProgressIndicator(
            Modifier.size(WontonTheme.size.iconSmall).semantics { contentDescription = label },
            color = WontonTheme.colors.textPrimary.toColor(), strokeWidth = WontonTheme.size.hairline,
        )
    }
}

@Composable
private fun ConnectedLine(account: String?) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // 装饰：右边就是“已接好”。
        Icon(WontonIcon.Check, null, Modifier.size(WontonTheme.size.iconSmall), tint = colors.success.toColor())
        Text(
            stringResource(R.string.agent_models_connected),
            style = WontonTheme.typography.body, color = colors.textPrimary.toColor(),
        )
        account?.let {
            Text(it, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
        }
    }
}

/** 断开要确认：存在这个助手里的 key 或登录会删掉，在用的是这家时顶上变回“还没接模型”。 */
@Composable
private fun DisconnectButton(page: ModelProviderPage, send: (AgentModelsAction) -> Unit) {
    var asking by remember { mutableStateOf(false) }
    WontonButton(
        stringResource(R.string.agent_models_disconnect), { asking = true },
        Modifier.fillMaxWidth(), kind = WontonButtonKind.DANGER, enabled = !page.busy,
    )
    if (!asking) return
    WontonDialog(
        title = stringResource(R.string.agent_models_disconnect_title, page.row.name),
        message = stringResource(R.string.agent_models_disconnect_message),
        onDismiss = { asking = false },
        actions = listOf(
            WontonDialogAction(stringResource(R.string.common_cancel)) { asking = false },
            WontonDialogAction(stringResource(R.string.agent_models_disconnect), WontonButtonKind.DANGER) {
                asking = false
                send(AgentModelsAction.DisconnectModelProvider)
            },
        ),
    )
}
