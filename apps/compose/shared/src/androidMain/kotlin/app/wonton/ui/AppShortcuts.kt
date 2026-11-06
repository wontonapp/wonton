package app.wonton.ui

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
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
import app.wonton.ui.icons.Agent
import app.wonton.ui.icons.AppsFill
import app.wonton.ui.icons.Compose
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.CancellationException

private const val PLUGIN_SHORTCUT_PREFIX = "plugin:"
private const val FIXED_SHORTCUT_COUNT = 2

@Composable
internal fun AppShortcutsEffect(tabs: TabsState) {
    val context = LocalContext.current.applicationContext
    val publisher = remember(context) { AppShortcuts(context) }
    val compose = rememberShortcutIcon(WontonIcon.Compose)
    val agent = rememberShortcutIcon(WontonIcon.Agent)
    val missing = rememberShortcutIcon(WontonIcon.AppsFill)
    val installed = tabs.plugins.viewModel.installed?.plugins
    val installedIds = installed?.mapTo(mutableSetOf()) { it.id }
    val pages = tabs.viewModel.items.mapNotNull { item ->
        val tab = item.tab as? TabRef.Plugin ?: return@mapNotNull null
        val plugin = installed?.firstOrNull { it.id == tab.pluginId }
        ShortcutPage(tab, item.page?.title ?: plugin?.title.orEmpty(), plugin?.icon?.small)
    }
    var resume by remember { mutableIntStateOf(0) }
    LifecycleResumeEffect(publisher) {
        resume++
        onPauseOrDispose {}
    }
    LaunchedEffect(publisher, pages, installedIds, compose, agent, missing, resume) {
        try {
            publisher.publish(tabs.plugins, pages, installedIds, compose, agent, missing)
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (failure: ShortcutPublishFailure) {
            tabs.plugins.reject(Problem("桌面快捷方式没有更新", failure.message))
        } catch (failure: IllegalArgumentException) {
            tabs.plugins.reject(Problem("桌面快捷方式没有更新", "系统没有接受快捷方式。", failure.toString()))
        } catch (failure: IllegalStateException) {
            tabs.plugins.reject(Problem("桌面快捷方式没有更新", "请解锁设备后重新打开云吞。", failure.toString()))
        } catch (failure: SecurityException) {
            tabs.plugins.reject(Problem("桌面快捷方式没有更新", "系统不允许更新快捷方式。", failure.toString()))
        }
    }
}

private class AppShortcuts(private val context: Context) {
    private val activity = ComponentName(context.packageName, "${context.packageName}.MainActivity")

    suspend fun publish(
        plugins: PluginsState, pages: List<ShortcutPage>, installed: Set<String>?,
        compose: IconCompat, agent: IconCompat, missing: IconCompat,
    ) {
        val existing = ShortcutManagerCompat.getShortcuts(
            context, ShortcutManagerCompat.FLAG_MATCH_DYNAMIC or ShortcutManagerCompat.FLAG_MATCH_PINNED,
        ).associateBy { it.id }.values
        val maximum = ShortcutManagerCompat.getMaxShortcutCountPerActivity(context)
        val fixed = listOf(
            shortcut("new-chat", "新会话", AppLink.NewChat, compose, 0),
            shortcut("agent-chat", "问 Bot", AppLink.AgentChat, agent, 1),
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
                    val icon = pluginIcon(plugins, page, missing)
                    dynamic += shortcut(shortcutId(page.tab), page.title, AppLink.Tab(page.tab), icon, dynamic.size)
                } catch (failure: ShortcutPublishFailure) {
                    problems += failure.message.orEmpty()
                }
            }
        }
        updateDynamic(dynamic)
        if (maximum < FIXED_SHORTCUT_COUNT) {
            problems += "系统只允许 $maximum 个快捷方式，不能同时显示“新会话”和“问 Bot”。"
        }
        if (installed != null && pages.size > maximum - fixed.size) {
            problems += "系统最多允许 $maximum 个快捷方式，已优先保留“新会话”“问 Bot”，插件页按底栏顺序显示。"
        }
        if (problems.isNotEmpty()) throw ShortcutPublishFailure(problems.joinToString("\n"))
    }

    private fun updateDynamic(shortcuts: List<ShortcutInfoCompat>) {
        if (ShortcutManagerCompat.isRateLimitingActive(context)) {
            throw ShortcutPublishFailure("系统暂时限制了快捷方式更新，请稍后重新打开云吞。")
        }
        if (!ShortcutManagerCompat.setDynamicShortcuts(context, shortcuts)) {
            throw ShortcutPublishFailure("系统没有接受快捷方式更新，请稍后重新打开云吞。")
        }
    }

    private fun disableUninstalled(existing: Collection<ShortcutInfoCompat>, ids: Set<String>) {
        val removed = existing.filter { shortcut ->
            shortcut.pluginTab()?.let { it.pluginId !in ids } == true
        }.map { it.id }
        if (removed.isNotEmpty()) ShortcutManagerCompat.disableShortcuts(context, removed, "这个应用已卸载")
        val restored = existing.filter { shortcut ->
            !shortcut.isEnabled && shortcut.pluginTab()?.let { it.pluginId in ids } == true
        }
        if (restored.isNotEmpty()) ShortcutManagerCompat.enableShortcuts(context, restored)
    }

    private suspend fun pluginIcon(plugins: PluginsState, page: ShortcutPage, missing: IconCompat): IconCompat {
        val source = page.icon ?: return missing
        val image = plugins.imageOrNull(source)
            ?: throw ShortcutPublishFailure("“${page.title}”的图标没有取到，请检查连接后重新打开云吞。")
        // ImageDecoder 可能返回硬件位图；系统快捷方式跨进程传图需要可序列化的软件位图。
        val original = image.asAndroidBitmap()
        val bitmap = if (original.config == Bitmap.Config.HARDWARE) {
            original.copy(Bitmap.Config.ARGB_8888, false)
                ?: throw ShortcutPublishFailure("“${page.title}”的图标无法转换，请重新打开云吞。")
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

private class ShortcutPublishFailure(message: String) : Exception(message)
