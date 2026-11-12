package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import app.wonton.protocol.CoreUpgradeCompat
import app.wonton.protocol.InstalledPlugins
import app.wonton.protocol.PluginIcon
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Refresh
import app.wonton.ui.icons.WarningFill
import app.wonton.ui.icons.WontonIcon

private val SPINNER_STROKE = 2.dp

/**
 * 核心升级页的兼容块（ADR-0085）：升级会新失效的应用列出来（不拦升级按钮），没查成的给一句轻提示，
 * 点一下重查。Ok 和不认识的类别不显示。原因详情由客户端核心按 App 的语言写好，进页面底部的“详情”。
 */

/** 升级会新失效的一份应用：名字和图标按已装列表显示，不在列表里（刚卸载等）就显示 ID 和占位图标。 */
internal data class CompatApp(val id: String, val title: String, val icon: PluginIcon?)

/** 目标版本下会新失效的已装应用；核心只报新失效的，这里不比较版本。 */
internal fun CoreUpgradeCompat.breakingApps(installed: InstalledPlugins?): List<CompatApp> =
    (this as? CoreUpgradeCompat.Breaking)?.plugins.orEmpty().map { plugin ->
        val known = installed?.plugins?.firstOrNull { it.id == plugin.id }
        CompatApp(plugin.id, known?.title ?: plugin.id, known?.icon)
    }

@Composable
internal fun CompatSection(
    compat: CoreUpgradeCompat,
    apps: List<CompatApp>,
    refreshing: Boolean,
    onRetry: () -> Unit,
    icon: @Composable (PluginIcon?) -> Unit,
) {
    when (compat) {
        is CoreUpgradeCompat.Breaking -> if (apps.isNotEmpty()) BreakingApps(apps, icon)
        is CoreUpgradeCompat.Failed, is CoreUpgradeCompat.Unavailable -> RetryNotice(refreshing, onRetry)
        else -> Unit
    }
}

/** 醒目但不吓人：警示色的图标和标题，下面列应用；升级按钮照常能用。 */
@Composable
private fun BreakingApps(apps: List<CompatApp>, icon: @Composable (PluginIcon?) -> Unit) {
    val colors = WontonTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                WontonIcon.WarningFill,
                // 装饰：标题把话说全了。
                contentDescription = null,
                Modifier.size(WontonTheme.size.iconSmall),
                tint = colors.warning.toColor(),
            )
            Text(
                stringResource(R.string.core_upgrade_compat_breaking_title),
                style = WontonTheme.typography.body,
                color = colors.warning.toColor(),
            )
        }
        apps.forEach { app ->
            Row(
                Modifier.padding(start = WontonTheme.size.iconSmall + WontonTheme.spacing.s8),
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                icon(app.icon)
                Text(app.title, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
            }
        }
    }
}

/** 没查到哪些应用会受影响：一行轻提示，点一下重查（重查时转圈）。 */
@Composable
private fun RetryNotice(refreshing: Boolean, onRetry: () -> Unit) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = WontonTheme.size.target)
            .clickable(enabled = !refreshing, role = Role.Button, onClick = onRetry),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (refreshing) {
            CircularProgressIndicator(Modifier.size(WontonTheme.size.iconSmall), strokeWidth = SPINNER_STROKE)
        } else {
            Icon(
                WontonIcon.Refresh,
                // 装饰：整行可点，图标只表示重查。
                contentDescription = null,
                Modifier.size(WontonTheme.size.iconSmall),
                tint = colors.textSecondary.toColor(),
            )
        }
        Text(
            stringResource(R.string.core_upgrade_compat_not_checked),
            style = WontonTheme.typography.body,
            color = colors.textSecondary.toColor(),
        )
    }
}
