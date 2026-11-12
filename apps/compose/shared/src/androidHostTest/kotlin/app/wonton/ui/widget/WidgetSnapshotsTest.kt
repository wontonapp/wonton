// 假快照的时间、未读数都是写死的字面量。
@file:Suppress("MagicNumber")

package app.wonton.ui.widget

import android.content.ComponentName
import android.content.Intent
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.compose.runtime.Composable
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.glance.appwidget.ExperimentalGlanceRemoteViewsApi
import androidx.glance.appwidget.GlanceRemoteViews
import androidx.glance.appwidget.action.actionStartActivity
import app.cash.paparazzi.DeviceConfig
import app.cash.paparazzi.Paparazzi
import app.wonton.protocol.BotActivity
import app.wonton.protocol.MessageSummary
import app.wonton.protocol.RoomPreview
import app.wonton.protocol.WidgetAccount
import app.wonton.protocol.WidgetAgent
import app.wonton.protocol.WidgetRoom
import app.wonton.protocol.WidgetSnapshot
import app.wonton.protocol.WidgetSnapshotRead
import app.wonton.ui.previewAppearance
import com.android.ide.common.rendering.api.SessionParams.RenderingMode
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import java.util.TimeZone

/**
 * 桌面小组件的参考图（ADR-0100）：Glance 先出 RemoteViews，再按系统的方式铺成 View 拍下来。
 * 只画写死的假快照，不读文件、不加载 .so；设备、语言、主题与 PreviewSnapshotsTest 一致。
 */
class WidgetSnapshotsTest {
    @get:Rule
    val paparazzi = Paparazzi(
        deviceConfig = DeviceConfig.PIXEL_5.copy(locale = "en", softButtons = false),
        theme = "android:Theme.Material.Light.NoActionBar.Fullscreen",
        // 只拍小组件那一块，不铺满整屏。
        renderingMode = RenderingMode.SHRINK,
        // 桌面铺 RemoteViews 用的是系统自己的 ImageView、TextView；换成 AppCompat 的，RemoteViews 不认它们的方法。
        appCompatEnabled = false,
    )

    private var zone: TimeZone? = null

    // “上次更新于”按系统时区写，固定成 UTC，参考图不随跑测试的机器变。
    @Before
    fun fixZone() {
        zone = TimeZone.getDefault()
        TimeZone.setDefault(TimeZone.getTimeZone("UTC"))
    }

    @After
    fun restoreZone() {
        TimeZone.setDefault(zone)
    }

    @Test
    fun recentChats() = snapshot(RECENT_CHATS) { RecentChats(it) }

    @Test
    fun recentChatsEmpty() = snapshot(RECENT_CHATS, signedIn().copy(rooms = emptyList(), unreadTotal = 0uL)) {
        RecentChats(it)
    }

    @Test
    fun recentChatsNotPaired() = snapshot(RECENT_CHATS, signedIn().copy(account = WidgetAccount.NOT_PAIRED)) {
        RecentChats(it)
    }

    @Test
    fun recentChatsSignedOut() = snapshot(RECENT_CHATS, signedIn().copy(account = WidgetAccount.SIGNED_OUT)) {
        RecentChats(it)
    }

    @Test
    fun agentStatus() = snapshot(AGENT_STATUS) { AgentStatus(it) }

    @Test
    fun agentStatusUnknown() = snapshot(
        AGENT_STATUS,
        signedIn().copy(defaultAgent = agent().copy(activity = null, unreadCount = 0uL)),
    ) { AgentStatus(it) }

    @Test
    fun agentStatusNoAgent() = snapshot(AGENT_STATUS, signedIn().copy(defaultAgent = null)) { AgentStatus(it) }

    @Test
    fun askAgent() = snapshot(ASK_AGENT) { AskAgent(it) }

    // GlanceRemoteViews 是 Glance 把组合变成 RemoteViews 的公开入口，标着实验性；只在测试里用。
    @OptIn(ExperimentalGlanceRemoteViewsApi::class)
    private fun snapshot(
        size: DpSize,
        snapshot: WidgetSnapshot = signedIn(),
        content: @Composable (WidgetState) -> Unit,
    ) {
        val context = paparazzi.context
        val appearance = previewAppearance()
        val state = WidgetState(
            read = WidgetSnapshotRead.Ready(snapshot),
            light = appearance.light,
            dark = appearance.dark,
            nowMs = NOW_MS,
            // 不经核心拼 `wonton://` 链接（测试不加载 .so），点了去哪不在图上。
            open = { actionStartActivity(Intent().setComponent(ComponentName(context.packageName, "Preview"))) },
        )
        val remoteViews = runBlocking {
            GlanceRemoteViews().compose(context, size) { content(state) }.remoteViews
        }
        val density = context.resources.displayMetrics.density
        val host = FrameLayout(context)
        host.layoutParams = ViewGroup.LayoutParams(
            (size.width.value * density).toInt(),
            (size.height.value * density).toInt(),
        )
        val widget: View = remoteViews.apply(context, host)
        val fill = FrameLayout.LayoutParams.MATCH_PARENT
        host.addView(widget, FrameLayout.LayoutParams(fill, fill))
        paparazzi.snapshot(host)
    }
}

// 桌面上 4×2、2×1、1×1 格的大小（Pixel 5 的默认桌面）。
private val RECENT_CHATS = DpSize(320.dp, 200.dp)
private val AGENT_STATUS = DpSize(200.dp, 72.dp)
private val ASK_AGENT = DpSize(80.dp, 80.dp)

// 2026-11-13 12:00 UTC；快照写在 5 分钟前。
private const val NOW_MS = 1_794_571_200_000L
private const val UPDATED_MS = NOW_MS - 5 * 60 * 1000

private fun preview(text: String, own: Boolean = false) = RoomPreview(
    senderName = if (own) "Me" else "Ann",
    isOwn = own,
    body = MessageSummary.Text(text),
    timestampMs = UPDATED_MS.toULong(),
)

private fun agent() = WidgetAgent(
    userId = "@hermes:home.example",
    name = "Hermes",
    activity = BotActivity.WORKING,
    unreadCount = 2uL,
)

private fun signedIn(): WidgetSnapshot {
    val appearance = previewAppearance()
    return WidgetSnapshot(
        version = 1u,
        updatedAtMs = UPDATED_MS.toULong(),
        account = WidgetAccount.SIGNED_IN,
        unreadTotal = 5uL,
        rooms = listOf(
            WidgetRoom("!family:home.example", "Family", false, 3uL, preview("Dinner at seven?")),
            WidgetRoom("!hermes:home.example", "Hermes", true, 2uL, preview("Here's the summary you asked for.")),
            WidgetRoom("!ann:home.example", "Ann", true, 0uL, preview("See you tomorrow", own = true)),
            WidgetRoom("!trip:home.example", "Trip planning", false, 0uL, null),
        ),
        defaultAgent = agent(),
        contacts = emptyList(),
        plugins = emptyList(),
        light = appearance.light,
        dark = appearance.dark,
    )
}
