package app.wonton.ui

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.drawable.IconCompat
import androidx.lifecycle.compose.LifecycleResumeEffect
import app.wonton.client.appLinkUrl
import app.wonton.client.parseAppLink
import app.wonton.protocol.AppLink
import app.wonton.protocol.PluginImage
import app.wonton.protocol.TabRef
import app.wonton.protocol.WidgetAccount
import app.wonton.ui.icons.Agent
import app.wonton.ui.icons.AppsFill
import app.wonton.ui.icons.Chat
import app.wonton.ui.icons.Compose
import app.wonton.ui.icons.WontonIcon
import app.wonton.ui.widget.widgetSnapshotUpdates
import kotlinx.coroutines.CancellationException

private const val PLUGIN_SHORTCUT_PREFIX = "plugin:"
private const val ROOM_SHORTCUT_PREFIX = "room:"
private const val FIXED_SHORTCUT_COUNT = 2

@Composable
internal fun AppShortcutsEffect(tabs: PagesState) {
    val context = LocalContext.current.applicationContext
    val publisher = remember(context) { AppShortcuts(context) }
    val compose = rememberShortcutIcon(WontonIcon.Compose)
    val agent = rememberShortcutIcon(WontonIcon.Agent)
    val missing = rememberShortcutIcon(WontonIcon.AppsFill)
    val chat = rememberShortcutIcon(WontonIcon.Chat)
    // 桌面搜索里的会话来自核心的系统入口快照（ADR-0100），和小组件同一份；还没写过快照时为空，沿用系统里上次的。
    val snapshot by remember(tabs.core) { tabs.core.widgetSnapshotUpdates() }.collectAsState(initial = null)
    // 只取快捷方式用得到的名字和 ID：未读数、预览变了不重发。
    val rooms = snapshot?.let { current ->
        if (current.account != WidgetAccount.SIGNED_IN) return@let emptyList()
        current.rooms.map { ShortcutRoom(it.roomId, it.name) }
    }
    val installed = tabs.plugins.viewModel.installed?.plugins
    val installedIds = installed?.mapTo(mutableSetOf()) { it.id }
    val pages = tabs.viewModel.stack.mapNotNull { open ->
        val tab = open.tab as? TabRef.Plugin ?: return@mapNotNull null
        val plugin = installed?.firstOrNull { it.id == tab.pluginId }
        ShortcutPage(tab, open.page?.title ?: plugin?.title.orEmpty(), plugin?.icon?.small)
    }
    var resume by remember { mutableIntStateOf(0) }
    LifecycleResumeEffect(publisher) {
        resume++
        onPauseOrDispose {}
    }
    LaunchedEffect(publisher, pages, installedIds, rooms, compose, agent, missing, chat, resume) {
        try {
            publisher.publish(tabs.plugins, pages, installedIds, rooms, ShortcutIcons(compose, agent, missing, chat))
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (failure: ShortcutPublishFailure) {
            tabs.plugins.reject(Problem(titleRes = R.string.shortcuts_not_updated, detail = failure.message))
        } catch (failure: IllegalArgumentException) {
            tabs.plugins.reject(
                Problem(titleRes = R.string.shortcuts_not_updated, nextStepRes = R.string.shortcuts_rejected,
                    detail = failure.toString()),
            )
        } catch (failure: IllegalStateException) {
            tabs.plugins.reject(
                Problem(titleRes = R.string.shortcuts_not_updated, nextStepRes = R.string.shortcuts_unlock,
                    detail = failure.toString()),
            )
        } catch (failure: SecurityException) {
            tabs.plugins.reject(
                Problem(titleRes = R.string.shortcuts_not_updated, nextStepRes = R.string.shortcuts_not_allowed,
                    detail = failure.toString()),
            )
        }
    }
}

private class AppShortcuts(private val context: Context) {
    private val activity = ComponentName(context.packageName, "${context.packageName}.MainActivity")

    suspend fun publish(
        plugins: PluginsState, pages: List<ShortcutPage>, installed: Set<String>?, rooms: List<ShortcutRoom>?,
        icons: ShortcutIcons,
    ) {
        val existing = ShortcutManagerCompat.getShortcuts(
            context, ShortcutManagerCompat.FLAG_MATCH_DYNAMIC or ShortcutManagerCompat.FLAG_MATCH_PINNED,
        ).associateBy { it.id }.values
        val maximum = ShortcutManagerCompat.getMaxShortcutCountPerActivity(context)
        val fixed = listOf(
            shortcut("new-chat", context.getString(R.string.shortcut_new_chat), AppLink.NewChat, icons.compose, 0),
            shortcut("agent-chat", context.getString(R.string.shortcut_agent_chat), AppLink.AgentChat, icons.agent, 1),
        )
        if (installed != null) disableUninstalled(existing, installed)
        val dynamic = fixed.take(maximum).toMutableList()
        val problems = mutableListOf<String>()
        if (installed == null) {
            // 首次列表未到不能当成卸载，也不能把系统里上次发布的插件项擦掉。
            dynamic += existing.filter { it.isDynamic && it.id.startsWith(PLUGIN_SHORTCUT_PREFIX) }
                .sortedBy { it.rank }.take((maximum - dynamic.size).coerceAtLeast(0))
        } else {
            for (page in pages.take((maximum - fixed.size).coerceAtLeast(0))) {
                if (page.tab.pluginId !in installed) continue
                try {
                    val icon = pluginIcon(plugins, page, icons.missing)
                    dynamic += shortcut(shortcutId(page.tab), page.title, AppLink.Tab(page.tab), icon, dynamic.size)
                } catch (failure: ShortcutPublishFailure) {
                    problems += failure.message.orEmpty()
                }
            }
        }
        dynamic += roomShortcuts(existing, rooms, icons.chat, first = dynamic.size, slots = maximum - dynamic.size)
        updateDynamic(dynamic)
        if (maximum < FIXED_SHORTCUT_COUNT) {
            problems += context.getString(R.string.shortcuts_too_few, maximum)
        }
        if (installed != null && pages.size > maximum - fixed.size) {
            problems += context.getString(R.string.shortcuts_too_many, maximum)
        }
        if (problems.isNotEmpty()) throw ShortcutPublishFailure(problems.joinToString("\n"))
    }

    /**
     * 固定的和底栏插件页之后剩下的位置给最近的会话，桌面长按和桌面搜索里都看得到（ADR-0100）。只放名字，不放消息。
     * 位置不够时少放几个，不算问题：会话本来就只放最近的。
     * [rooms] 为 null 表示还不知道（核心还没写过快照），沿用系统里上次发的。
     */
    private fun roomShortcuts(
        existing: Collection<ShortcutInfoCompat>, rooms: List<ShortcutRoom>?, icon: IconCompat, first: Int, slots: Int,
    ): List<ShortcutInfoCompat> {
        val count = slots.coerceAtLeast(0)
        return if (rooms == null) {
            existing.filter { it.isDynamic && it.id.startsWith(ROOM_SHORTCUT_PREFIX) }.sortedBy { it.rank }.take(count)
        } else {
            // 系统不收空的标题。
            rooms.filter { it.name.isNotBlank() }.take(count).mapIndexed { index, room ->
                shortcut(ROOM_SHORTCUT_PREFIX + room.roomId, room.name, AppLink.Room(room.roomId), icon, first + index)
            }
        }
    }

    private fun updateDynamic(shortcuts: List<ShortcutInfoCompat>) {
        if (ShortcutManagerCompat.isRateLimitingActive(context)) {
            throw ShortcutPublishFailure(context.getString(R.string.shortcuts_rate_limited))
        }
        if (!ShortcutManagerCompat.setDynamicShortcuts(context, shortcuts)) {
            throw ShortcutPublishFailure(context.getString(R.string.shortcuts_update_refused))
        }
    }

    private fun disableUninstalled(existing: Collection<ShortcutInfoCompat>, ids: Set<String>) {
        val removed = existing.filter { shortcut ->
            shortcut.pluginTab()?.let { it.pluginId !in ids } == true
        }.map { it.id }
        if (removed.isNotEmpty()) {
            val reason = context.getString(R.string.shortcuts_app_uninstalled)
            ShortcutManagerCompat.disableShortcuts(context, removed, reason)
        }
        val restored = existing.filter { shortcut ->
            !shortcut.isEnabled && shortcut.pluginTab()?.let { it.pluginId in ids } == true
        }
        if (restored.isNotEmpty()) ShortcutManagerCompat.enableShortcuts(context, restored)
    }

    private suspend fun pluginIcon(plugins: PluginsState, page: ShortcutPage, missing: IconCompat): IconCompat {
        val source = page.icon ?: return missing
        val image = plugins.imageOrNull(source)
            ?: throw ShortcutPublishFailure(context.getString(R.string.shortcut_icon_missing, page.title))
        // ImageDecoder 可能返回硬件位图；系统快捷方式跨进程传图需要可序列化的软件位图。
        val original = image.asAndroidBitmap()
        val bitmap = if (original.config == Bitmap.Config.HARDWARE) {
            original.copy(Bitmap.Config.ARGB_8888, false)
                ?: throw ShortcutPublishFailure(context.getString(R.string.shortcut_icon_convert_failed, page.title))
        } else original
        return IconCompat.createWithBitmap(bitmap)
    }

    private fun shortcut(id: String, title: String, link: AppLink, icon: IconCompat, rank: Int): ShortcutInfoCompat =
        ShortcutInfoCompat.Builder(context, id)
            .setShortLabel(title)
            .setLongLabel(title)
            .setIcon(icon)
            .setRank(rank)
            .setActivity(activity)
            .setIntent(Intent(Intent.ACTION_VIEW, Uri.parse(appLinkUrl(link)))
                .setComponent(activity).addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP))
            .build()
}

private fun shortcutId(tab: TabRef.Plugin): String = "$PLUGIN_SHORTCUT_PREFIX${appLinkUrl(AppLink.Tab(tab))}"

private fun ShortcutInfoCompat.pluginTab(): TabRef.Plugin? {
    if (!id.startsWith(PLUGIN_SHORTCUT_PREFIX)) return null
    return (parseAppLink(id.removePrefix(PLUGIN_SHORTCUT_PREFIX)) as? AppLink.Tab)?.tab as? TabRef.Plugin
}

private data class ShortcutPage(val tab: TabRef.Plugin, val title: String, val icon: PluginImage?)

private data class ShortcutRoom(val roomId: String, val name: String)

private class ShortcutIcons(
    val compose: IconCompat,
    val agent: IconCompat,
    val missing: IconCompat,
    val chat: IconCompat,
)

/**
 * 没登录了（退出、被吊销、换服务器）：桌面长按菜单和桌面搜索里的会话马上撤掉，不等 App 打开。用户自己固定到桌面的
 * 快捷方式系统不让 App 删，点了进 App 由 App 说明。
 */
fun removeRoomShortcuts(context: Context) {
    val ids = ShortcutManagerCompat.getDynamicShortcuts(context).map { it.id }
        .filter { it.startsWith(ROOM_SHORTCUT_PREFIX) }
    if (ids.isNotEmpty()) ShortcutManagerCompat.removeDynamicShortcuts(context, ids)
}

private class ShortcutPublishFailure(message: String) : Exception(message)
