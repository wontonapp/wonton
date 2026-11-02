package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.CoreException
import app.wonton.protocol.InstallPreview

/** 安装、升级的确认页：原样显示家里的服务器给的预览，确认后把这份预览交回核心，授予的就是这里列出的权限。 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun InstallConfirmScreen(plugins: PluginsState, confirmation: PluginConfirmation, onClose: () -> Unit) {
    val isUpgrade = confirmation is PluginConfirmation.Upgrade
    var attempt by remember { mutableIntStateOf(0) }
    var rejection by remember { mutableStateOf<String?>(null) }
    val loaded by produceState<Result<InstallPreview>?>(null, confirmation, attempt) {
        value = null
        value = try {
            Result.success(
                when (confirmation) {
                    is PluginConfirmation.Install -> plugins.core.previewInstall(confirmation.source)
                    is PluginConfirmation.Upgrade -> plugins.core.previewUpgrade(confirmation.id)
                },
            )
        } catch (failure: CoreException) {
            Result.failure(failure)
        }
    }
    // 装不了（blocked）或者有核心不支持的权限时不能确认。
    val preview = loaded?.getOrNull()?.takeIf { it.blocked == null && it.permissions.all { line -> line.supported } }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(if (isUpgrade) "升级" else "安装") },
                navigationIcon = { TextButton(onClick = onClose) { Text("取消") } },
                actions = {
                    TextButton(
                        onClick = {
                            // 在这里直接发给核心：被拒绝时原因留在确认页上，不随确认页关掉。
                            preview?.let {
                                val action = if (isUpgrade) Action.UpgradePlugin(it) else Action.InstallPlugin(it)
                                rejection = plugins.core.send(action)
                                if (rejection == null) onClose()
                            }
                        },
                        enabled = preview != null,
                    ) { Text(if (isUpgrade) "升级" else "安装") }
                },
            )
        },
    ) { padding ->
        val result = loaded
        Box(Modifier.padding(padding).fillMaxSize()) {
            when {
                result == null -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                result.isFailure -> ProblemView(result.exceptionOrNull().toString(), null, onRetry = { attempt++ })
                else -> PreviewContent(result.getOrThrow(), plugins)
            }
        }
    }
    rejection?.let { reason ->
        AlertDialog(
            onDismissRequest = { rejection = null },
            confirmButton = { TextButton(onClick = { rejection = null }) { Text("好") } },
            title = { Text("操作被拒绝") },
            text = { Text(reason) },
        )
    }
}

@Composable
private fun PreviewContent(preview: InstallPreview, plugins: PluginsState) {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(14.dp), verticalAlignment = Alignment.CenterVertically) {
            PluginIconImage(preview.icon, preview.title, 64.dp, plugins)
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(preview.title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                val version = preview.upgradeFrom?.let { "$it → ${preview.version}" } ?: preview.version
                Text(version, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Row(
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Tag(preview.trust.label, preview.trust.tint)
                    preview.author?.let {
                        Text(
                            it.name,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
        preview.blocked?.let { Text(it, color = Red) }
        if (preview.notices.isNotEmpty()) NoticeLines(preview.notices)
        if (preview.permissions.isNotEmpty()) {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("权限", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                preview.permissions.forEach { PermissionRow(it) }
            }
        }
        preview.releaseNotes?.let { notes ->
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("更新说明", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                Text(notes)
            }
        }
    }
}
