package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import app.wonton.protocol.Action
import app.wonton.protocol.CoreException
import app.wonton.protocol.InstallPreview
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Blocked
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.WontonIcon

/** 安装、升级的确认页：原样显示家里的服务器给的预览，确认后把这份预览交回核心，授予的就是这里列出的权限。 */
@Composable
fun InstallConfirmScreen(plugins: PluginsState, confirmation: PluginConfirmation, onClose: () -> Unit) {
    val isUpgrade = confirmation is PluginConfirmation.Upgrade
    var attempt by remember { mutableIntStateOf(0) }
    var rejection by remember { mutableStateOf<String?>(null) }
    var lanWebui by remember { mutableStateOf(false) }
    val loaded by produceState<Result<InstallPreview>?>(null, confirmation, attempt) {
        value = null
        value = try {
            Result.success(
                when (confirmation) {
                    is PluginConfirmation.Install -> plugins.core.previewInstall(confirmation.source)
                    is PluginConfirmation.Upgrade -> plugins.core.previewUpgrade(confirmation.id)
                }.also { lanWebui = it.lanWebui?.enabled ?: false },
            )
        } catch (failure: CoreException) {
            Result.failure(failure)
        }
    }
    val title = if (isUpgrade) "升级" else "安装"
    WontonScreen(bar = { WontonPushedBar(title, onClose, WontonIcon.Close, "关闭") }) {
        val result = loaded
        val preview = result?.getOrNull()
        Column(Modifier.fillMaxSize()) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                when {
                    result == null -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                    result.isFailure -> ProblemView(result.exceptionOrNull().toString(), null, onRetry = { attempt++ })
                    preview != null -> PreviewContent(preview, plugins, lanWebui) { lanWebui = it }
                }
            }
            preview?.let {
                WontonButton(
                    title,
                    {
                        // 在这里直接发给核心：被拒绝时原因留在确认页上，不随确认页关掉。
                        val shown = it.copy(lanWebui = it.lanWebui?.copy(enabled = lanWebui))
                        val action = if (isUpgrade) Action.UpgradePlugin(shown) else Action.InstallPlugin(shown)
                        rejection = plugins.core.send(action)
                        if (rejection == null) onClose()
                    },
                    Modifier.windowInsetsPadding(WindowInsets.navigationBars)
                        .padding(WontonTheme.spacing.pageInset)
                        .fillMaxWidth(),
                    // 装不了（blocked）或者有核心不支持的权限时不能确认。
                    enabled = it.blocked == null && it.permissions.all { line -> line.supported },
                )
            }
        }
    }
    rejection?.let { WontonDialog("操作被拒绝", it, { rejection = null }) }
}

@Composable
private fun PreviewContent(
    preview: InstallPreview,
    plugins: PluginsState,
    lanWebui: Boolean,
    onLanWebui: (Boolean) -> Unit,
) {
    val colors = WontonTheme.colors
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            PluginIconImage(preview.icon, preview.category, WontonTheme.size.previewIcon, plugins)
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                Text(preview.title, style = WontonTheme.typography.itemTitle, color = colors.textPrimary.toColor())
                val version = preview.upgradeFrom?.let { "$it → ${preview.version}" } ?: preview.version
                Text(version, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
                Row(
                    horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Tag(preview.trust.label, preview.trust.tint)
                    preview.author?.let {
                        Text(it.name, style = WontonTheme.typography.caption, color = colors.textSecondary.toColor())
                    }
                }
            }
        }
        preview.blocked?.let { WontonNotice(WontonIcon.Blocked, it) }
        if (preview.notices.isNotEmpty()) NoticeLines(preview.notices)
        if (preview.permissions.isNotEmpty()) {
            DetailSection("权限") { preview.permissions.forEach { PermissionRow(it) } }
        }
        preview.lanWebui?.let { LanWebuiToggle(it, lanWebui, enabled = true, onLanWebui) }
        preview.lanWebui?.let { LanWebuiToggle(it, lanWebui, enabled = true, onLanWebui) }
        preview.releaseNotes?.let { notes ->
            DetailSection("更新说明") {
                Text(notes, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
            }
        }
    }
}
