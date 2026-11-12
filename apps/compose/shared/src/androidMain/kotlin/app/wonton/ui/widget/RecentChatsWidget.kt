package app.wonton.ui.widget

import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.unit.dp
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.LocalContext
import androidx.glance.LocalSize
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.SizeMode
import androidx.glance.appwidget.provideContent
import androidx.glance.layout.Alignment
import androidx.glance.layout.Column
import androidx.glance.layout.Row
import androidx.glance.layout.Spacer
import androidx.glance.layout.fillMaxSize
import androidx.glance.layout.fillMaxWidth
import androidx.glance.layout.height
import androidx.glance.layout.padding
import androidx.glance.layout.width
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextStyle
import app.wonton.protocol.AppLink
import app.wonton.protocol.WidgetRoom
import app.wonton.ui.R
import app.wonton.ui.design.generated.WontonSpacing
import app.wonton.ui.line

/** 最近会话/未读（ADR-0100）：最近的几个会话和未读数，点一行进那个会话。 */
class RecentChatsWidget : GlanceAppWidget() {
    // 行数按小组件此刻的高度算，拉高了多放几行。
    override val sizeMode: SizeMode = SizeMode.Exact

    override suspend fun provideGlance(context: Context, id: GlanceId) {
        provideContent {
            val revision by WidgetRevision.flow.collectAsState()
            val state = remember(revision) { loadWidgetState(context) }
            RecentChats(state)
        }
    }
}

class RecentChatsWidgetReceiver : GlanceAppWidgetReceiver() {
    override val glanceAppWidget: GlanceAppWidget = RecentChatsWidget()
}

/** 一行（名字加一行预览）的高度，按它算放得下几行。 */
private val ROW_HEIGHT = 44.dp
private val FOOTER_HEIGHT = 20.dp

@Composable
fun RecentChats(state: WidgetState) {
    val context = LocalContext.current
    WidgetFrame(state, chatList) {
        val snapshot = state.snapshot
        val notice = accountNotice(state)
        if (snapshot == null || notice != null) {
            WidgetNotice(state, notice.orEmpty())
            return@WidgetFrame
        }
        Column(GlanceModifier.fillMaxSize().padding(horizontal = WontonSpacing.s12, vertical = WontonSpacing.s8)) {
            if (snapshot.rooms.isEmpty()) {
                Text(
                    context.getString(R.string.chat_empty),
                    GlanceModifier.defaultWeight().fillMaxWidth(),
                    style = TextStyle(color = state.colors.textSecondary, fontSize = widgetType.supporting.fontSize),
                )
            } else {
                val space = LocalSize.current.height - WontonSpacing.s8 * 2 - FOOTER_HEIGHT
                val fits = (space / ROW_HEIGHT).toInt().coerceAtLeast(1)
                Column(GlanceModifier.defaultWeight().fillMaxWidth()) {
                    snapshot.rooms.take(fits).forEach { RoomLine(state, it) }
                }
            }
            Text(
                context.getString(R.string.widget_updated_at, updatedAt(context, state, snapshot.updatedAtMs.toLong())),
                GlanceModifier.fillMaxWidth().height(FOOTER_HEIGHT),
                style = TextStyle(color = state.colors.textSecondary, fontSize = widgetType.caption.fontSize),
                maxLines = 1,
            )
        }
    }
}

@Composable
private fun RoomLine(state: WidgetState, room: WidgetRoom) {
    val resources = LocalContext.current.resources
    val preview = room.preview?.let { preview ->
        val line = preview.body.line(resources)
        if (preview.isOwn) resources.getString(R.string.chat_own_preview, line) else line
    }
    val unread = room.unreadCount > 0uL
    Row(
        GlanceModifier.fillMaxWidth().height(ROW_HEIGHT).clickable(state.open(AppLink.Room(room.roomId))),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(GlanceModifier.defaultWeight()) {
            Text(
                room.name,
                style = TextStyle(
                    color = state.colors.textPrimary,
                    fontSize = widgetType.body.fontSize,
                    fontWeight = if (unread) FontWeight.Bold else FontWeight.Medium,
                ),
                maxLines = 1,
            )
            if (preview != null) {
                Text(
                    preview,
                    style = TextStyle(color = state.colors.textSecondary, fontSize = widgetType.supporting.fontSize),
                    maxLines = 1,
                )
            }
        }
        if (unread) {
            Spacer(GlanceModifier.width(WontonSpacing.s8))
            UnreadBadge(state, room.unreadCount)
        }
    }
}
