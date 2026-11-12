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
        plugins: PluginsState, pages: List<ShortcutPage>, installed: Set<String>?,
        compose: IconCompat, agent: IconCompat, missing: IconCompat,
    ) {
        val existing = ShortcutManagerCompat.getShortcuts(
            context, ShortcutManagerCompat.FLAG_MATCH_DYNAMIC or ShortcutManagerCompat.FLAG_MATCH_PINNED,
        ).associateBy { it.id }.values
        val maximum = ShortcutManagerCompat.getMaxShortcutCountPerActivity(context)
        val fixed = listOf(
            shortcut("new-chat", context.getString(R.string.shortcut_new_chat), AppLink.NewChat, compose, 0),
            shortcut("agent-chat", context.getString(R.string.shortcut_agent_chat), AppLink.AgentChat, agent, 1),
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
            problems += context.getString(R.string.shortcuts_too_few, maximum)
        }
        if (installed != null && pages.size > maximum - fixed.size) {
            problems += context.getString(R.string.shortcuts_too_many, maximum)
        }
        if (problems.isNotEmpty()) throw ShortcutPublishFailure(problems.joinToString("\n"))
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

private class ShortcutPublishFailure(message: String) : Exception(message)
