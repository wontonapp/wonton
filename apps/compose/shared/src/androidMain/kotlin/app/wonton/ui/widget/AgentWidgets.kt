package app.wonton.ui.widget

import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.glance.ColorFilter
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.Image
import androidx.glance.ImageProvider
import androidx.glance.LocalContext
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.provideContent
import androidx.glance.layout.Alignment
import androidx.glance.layout.Column
import androidx.glance.layout.Row
import androidx.glance.layout.Spacer
import androidx.glance.layout.fillMaxSize
import androidx.glance.layout.height
import androidx.glance.layout.padding
import androidx.glance.layout.size
import androidx.glance.layout.width
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextAlign
import androidx.glance.text.TextStyle
import app.wonton.protocol.AppLink
import app.wonton.protocol.WidgetAgent
import app.wonton.ui.R
import app.wonton.ui.design.generated.WontonSize
import app.wonton.ui.design.generated.WontonSpacing
import app.wonton.ui.labelRes

/** 默认 Agent 的状态（ADR-0100）：名字、在做什么、私聊的未读数，点了进和它的私聊。固定默认 Agent，不做配置。 */
class AgentStatusWidget : GlanceAppWidget() {
    override suspend fun provideGlance(context: Context, id: GlanceId) {
        provideContent {
            val revision by WidgetRevision.flow.collectAsState()
            val state = remember(revision) { loadWidgetState(context) }
            AgentStatus(state)
        }
    }
}

class AgentStatusWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = AgentStatusWidget()
}

/** 快捷问 Bot（ADR-0100）：一个按钮，点了进和默认 Agent 的私聊、在输入框里接着说。快照只用来取颜色。 */
class AskAgentWidget : GlanceAppWidget() {
    override suspend fun provideGlance(context: Context, id: GlanceId) {
        provideContent {
            val revision by WidgetRevision.flow.collectAsState()
            val state = remember(revision) { loadWidgetState(context) }
            AskAgent(state)
        }
    }
}

class AskAgentWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = AskAgentWidget()
}

@Composable
fun AgentStatus(state: WidgetState) {
    val context = LocalContext.current
    WidgetFrame(state, AppLink.AgentChat) {
        val snapshot = state.snapshot
        val notice = accountNotice(state)
        val agent = snapshot?.defaultAgent
        when {
            snapshot == null || notice != null -> WidgetNotice(state, notice.orEmpty())
            agent == null -> WidgetNotice(state, context.getString(R.string.widget_choose_agent))
            else -> AgentLine(state, agent, updatedAt(context, state, snapshot.updatedAtMs.toLong()))
        }
    }
}

@Composable
private fun AgentLine(state: WidgetState, agent: WidgetAgent, updated: String) {
    val context = LocalContext.current
    val activity = agent.activity?.let { context.getString(it.labelRes()) }
        ?: context.getString(R.string.widget_agent_status_unknown)
    Row(
        GlanceModifier.fillMaxSize().padding(horizontal = WontonSpacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        AgentIcon(state, WontonSize.icon)
        Spacer(GlanceModifier.width(WontonSpacing.s8))
        Column(GlanceModifier.defaultWeight()) {
            Text(
                agent.name,
                style = TextStyle(
                    color = state.colors.textPrimary,
                    fontSize = widgetType.body.fontSize,
                    fontWeight = FontWeight.Medium,
                ),
                maxLines = 1,
            )
            Text(
                context.getString(R.string.widget_status_at, activity, updated),
                style = TextStyle(color = state.colors.textSecondary, fontSize = widgetType.caption.fontSize),
                maxLines = 1,
            )
        }
        if (agent.unreadCount > 0uL) {
            Spacer(GlanceModifier.width(WontonSpacing.s8))
            UnreadBadge(state, agent.unreadCount)
        }
    }
}

@Composable
fun AskAgent(state: WidgetState) {
    val context = LocalContext.current
    WidgetFrame(state, AppLink.AgentChat) {
        Column(
            GlanceModifier.fillMaxSize().padding(WontonSpacing.s4),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            AgentIcon(state, ASK_ICON)
            Spacer(GlanceModifier.height(WontonSpacing.s4))
            Text(
                context.getString(R.string.shortcut_agent_chat),
                style = TextStyle(
                    color = state.colors.textPrimary,
                    fontSize = widgetType.caption.fontSize,
                    textAlign = TextAlign.Center,
                ),
                maxLines = 1,
            )
        }
    }
}

private val ASK_ICON = 28.dp

@Composable
private fun AgentIcon(state: WidgetState, size: Dp) {
    Image(
        ImageProvider(R.drawable.widget_agent),
        contentDescription = null,
        modifier = GlanceModifier.size(size),
        colorFilter = ColorFilter.tint(state.colors.accentInk),
    )
}
