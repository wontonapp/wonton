package app.wonton.ui.widget

import android.content.Context
import android.text.format.DateUtils
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.dp
import androidx.glance.GlanceModifier
import androidx.glance.LocalContext
import androidx.glance.action.clickable
import androidx.glance.appwidget.appWidgetBackground
import androidx.glance.appwidget.cornerRadius
import androidx.glance.background
import androidx.glance.layout.Alignment
import androidx.glance.layout.Box
import androidx.glance.layout.fillMaxSize
import androidx.glance.layout.padding
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextAlign
import androidx.glance.text.TextStyle
import app.wonton.protocol.AppLink
import app.wonton.protocol.TabRef
import app.wonton.protocol.WidgetAccount
import app.wonton.protocol.WidgetSnapshotRead
import app.wonton.ui.R
import app.wonton.ui.design.generated.WontonRadii
import app.wonton.ui.design.generated.WontonSpacing
import app.wonton.ui.design.generated.WontonTypography
import java.time.Instant
import java.time.ZoneId

/** 字号取设计系统的令牌（ADR-0026）；Glance 不认 Compose 的字体，只取大小。 */
internal val widgetType = WontonTypography(null)

/** 小组件的底：App 的卡片色和圆角，整块可点。 */
@Composable
internal fun WidgetFrame(state: WidgetState, link: AppLink, content: @Composable () -> Unit) {
    Box(
        GlanceModifier.fillMaxSize()
            .appWidgetBackground()
            .background(state.colors.surface)
            .cornerRadius(WontonRadii.card)
            .clickable(state.open(link)),
        contentAlignment = Alignment.CenterStart,
    ) {
        content()
    }
}

/**
 * 没有会话可画时的一句话：没配对、登录失效、刚换服务器、还没打开过 App、文件读不了。点了开 App，由 App 接着说。
 * 已登录时为空。
 */
@Composable
internal fun accountNotice(state: WidgetState): String? {
    val context = LocalContext.current
    val read = state.read
    val res = when (read) {
        is WidgetSnapshotRead.Missing -> R.string.widget_open_to_start
        is WidgetSnapshotRead.Unreadable -> R.string.widget_open_to_refresh
        is WidgetSnapshotRead.Ready -> when (read.snapshot.account) {
            WidgetAccount.NOT_PAIRED -> R.string.widget_open_to_pair
            WidgetAccount.SIGNED_OUT -> R.string.widget_open_to_sign_in
            WidgetAccount.SWITCHING -> R.string.widget_open_to_refresh
            WidgetAccount.SIGNED_IN -> return null
        }
    }
    return context.getString(res)
}

@Composable
internal fun WidgetNotice(state: WidgetState, text: String) {
    // RemoteViews 的 TextView 不会在铺满的高度里竖直居中，靠外面一层 Box 放到中间。
    Box(GlanceModifier.fillMaxSize().padding(WontonSpacing.s12), contentAlignment = Alignment.Center) {
        Text(
            text,
            style = TextStyle(
                color = state.colors.textSecondary,
                fontSize = widgetType.supporting.fontSize,
                textAlign = TextAlign.Center,
            ),
            maxLines = 3,
        )
    }
}

/** 未读数的小圆标。 */
@Composable
internal fun UnreadBadge(state: WidgetState, count: ULong) {
    Text(
        // unread 是 ULong，显示和读屏都按十进制原样。
        count.toString(),
        GlanceModifier.background(state.colors.accentFill)
            .cornerRadius(WontonRadii.pill)
            .padding(horizontal = WontonSpacing.s8, vertical = 2.dp),
        style = TextStyle(
            color = state.colors.accentInkOnFill,
            fontSize = widgetType.caption.fontSize,
            fontWeight = FontWeight.Medium,
        ),
        maxLines = 1,
    )
}

/**
 * 快照写下的时刻：当天只写时分，早于当天带日期。小组件不会自己走时，写成绝对时间才一直是对的（ADR-0100 决策 3）。
 */
internal fun updatedAt(context: Context, state: WidgetState, updatedAtMs: Long): String {
    val zone = ZoneId.systemDefault()
    val day = { ms: Long -> Instant.ofEpochMilli(ms).atZone(zone).toLocalDate() }
    val flags = if (day(updatedAtMs) == day(state.nowMs)) {
        DateUtils.FORMAT_SHOW_TIME
    } else {
        DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_SHOW_TIME or DateUtils.FORMAT_ABBREV_MONTH
    }
    return DateUtils.formatDateTime(context, updatedAtMs, flags)
}

/** 默认打开的地方：聊天列表。 */
internal val chatList: AppLink = AppLink.Tab(TabRef.Chat)
