package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.AgentModelsAction
import app.wonton.protocol.AgentModelsState
import app.wonton.protocol.AgentModelsViewModel
import app.wonton.protocol.BotRef

/**
 * 画模型页要的东西：核心的 ViewModel、发动作的口子，和取核心此刻的值的口子。
 * 回到前台那一下组合里的 [view] 还没跟上，判断“浏览器是不是被关掉了”要看 [latest]。
 */
class AgentModels(
    val bot: BotRef,
    val view: AgentModelsViewModel,
    val latest: () -> AgentModelsViewModel,
    val send: (AgentModelsAction) -> Unit,
    val browser: ModelBrowser,
)

/**
 * 一个 Bot 的模型页（ADR-0129）的宿主：订阅核心、进来时打开这个 Bot、按登录的进展开关 App 内浏览器。
 * 页面状态全在核心，这里只留“核心拒了这次点击”的弹窗。
 */
@Composable
internal fun AgentModelsHost(core: ClientCore, bot: BotRef, content: @Composable (AgentModels) -> Unit) {
    val current by remember(core) { core.agentModelsUpdates() }.collectAsState(remember(core) { core.agentModels() })
    var rejection by remember { mutableStateOf<Problem?>(null) }
    val send = { action: AgentModelsAction -> rejection = core.send(Action.AgentModels(action)) }
    LaunchedEffect(core, bot) { send(AgentModelsAction.OpenAgentModels(bot)) }
    // 核心还看着别的 Bot（刚切过来）：当成正在取，不把别人的服务商画出来。
    val view = if (current.bot == bot) current else AgentModelsViewModel(
        bot = bot, state = AgentModelsState.Loading, current = null, test = null,
        recommended = emptyList(), more = emptyList(), search = "", provider = null, actionProblem = null,
    )
    val context = LocalContext.current
    val browser = remember(context) { ModelBrowser(context) { rejection = it } }
    val models = AgentModels(bot, view, latest = { core.agentModels() }, send = send, browser = browser)
    ModelLoginBrowserEffect(models)
    content(models)
    rejection?.let { ProblemDialog(it) { rejection = null } }
}
