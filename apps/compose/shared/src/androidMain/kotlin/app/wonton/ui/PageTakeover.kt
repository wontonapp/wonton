package app.wonton.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.PluginAction
import app.wonton.protocol.Action
import app.wonton.protocol.PluginFailure
import app.wonton.protocol.PluginTakeover
import app.wonton.protocol.TakeoverState
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

/** 插件网页上的“我来操作”（ADR-0024）。只有声明了 MCP 的插件才有。 */
class PageTakeover(private val pluginId: String, current: PluginTakeover?, private val send: (Action) -> Unit) {
    val state: TakeoverState? = current?.takeIf { it.id == pluginId }?.state

    fun start() = send(Action.Plugins(PluginAction.StartTakeover(pluginId)))

    fun end() = send(Action.Plugins(PluginAction.EndTakeover(pluginId)))
}

@Composable
fun TakeoverButton(takeover: PageTakeover) {
    when (takeover.state) {
        is TakeoverState.Operating -> TextButton(onClick = takeover::end) {
            Text(stringResource(R.string.takeover_hand_back))
        }
        is TakeoverState.Starting -> TextButton(onClick = {}, enabled = false) {
            Text(stringResource(R.string.takeover_start))
        }
        else -> TextButton(onClick = takeover::start) { Text(stringResource(R.string.takeover_start)) }
    }
}

/** 没开始成或者中途断了：Bot 的调用已经不停住了，要让主人知道。 */
@Composable
fun TakeoverNotice(takeover: PageTakeover) {
    val problem = takeover.state?.notice() ?: return
    Box(Modifier.padding(WontonTheme.spacing.s8)) {
        WontonNotice(WontonIcon.Warning, problem.titleText, lines = listOfNotNull(problem.nextStepText)) {
            problem.detail?.let { ProblemDetail(it) }
            WontonSmallButton(stringResource(R.string.takeover_got_it), takeover::end)
        }
    }
}

@Composable
private fun TakeoverState.notice(): Problem? = when (this) {
    is TakeoverState.Starting, is TakeoverState.Operating -> null
    is TakeoverState.NotStarted -> {
        val plain = problem.reason.problem(problem.message)
        Problem(stringResource(R.string.takeover_not_started, plain.titleText), plain.nextStepText, plain.detail)
    }
    is TakeoverState.Lost -> if (problem.reason == PluginFailure.TRANSPORT_UNAVAILABLE) {
        Problem(stringResource(R.string.takeover_lost), detail = problem.message)
    } else {
        val plain = problem.reason.problem(problem.message)
        Problem(stringResource(R.string.takeover_lost_reason, plain.titleText), plain.nextStepText, plain.detail)
    }
}
