package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.PluginFailure
import app.wonton.protocol.PluginOperation
import app.wonton.protocol.PluginTask
import app.wonton.protocol.PluginTaskState

/** 主屏或商店里打开的详情页、确认页和卸载确认。[open] 打开已安装的插件。 */
class PluginNavigation(val open: (PluginEntry) -> Unit) {
    var detail by mutableStateOf<PluginRoute?>(null)
    var confirmation by mutableStateOf<PluginConfirmation?>(null)
    var uninstalling by mutableStateOf<UninstallTarget?>(null)
}

/** [list] 上叠着打开的详情页和确认页，返回键逐层关掉。 */
@Composable
fun PluginPages(plugins: PluginsState, navigation: PluginNavigation, list: @Composable () -> Unit) {
    val confirmation = navigation.confirmation
    val detail = navigation.detail
    when {
        confirmation != null -> {
            BackHandler { navigation.confirmation = null }
            InstallConfirmScreen(plugins, confirmation, onClose = { navigation.confirmation = null })
        }
        detail != null -> {
            BackHandler { navigation.detail = null }
            PluginDetailScreen(plugins, navigation, detail)
        }
        else -> list()
    }
    navigation.uninstalling?.let { target ->
        UninstallDialog(target, plugins, onDismiss = { navigation.uninstalling = null })
    }
}

/** 卸载前确认：可以保留数据，也可以连数据一起删。 */
@Composable
private fun UninstallDialog(target: UninstallTarget, plugins: PluginsState, onDismiss: () -> Unit) {
    val uninstall = { deleteData: Boolean ->
        onDismiss()
        plugins.send(Action.UninstallPlugin(target.id, deleteData))
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("卸载“${target.title}”？") },
        text = { Text("删除数据会一并删掉它的密钥和 Bot 账号，不能恢复。") },
        confirmButton = {
            Column(horizontalAlignment = Alignment.End) {
                TextButton(onClick = { uninstall(false) }) { Text("卸载，保留数据") }
                TextButton(onClick = { uninstall(true) }) { Text("卸载并删除数据", color = Red) }
                TextButton(onClick = onDismiss) { Text("取消") }
            }
        },
    )
}

/** 安装、升级、卸载的一项进展。插件包在确认之后变了的，[onReview] 重新打开确认页。 */
@Composable
fun TaskRow(task: PluginTask, plugins: PluginsState, onReview: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        PluginIconImage(task.icon, task.title, 40.dp, plugins)
        Column(Modifier.weight(1f)) {
            Text(task.title, fontWeight = FontWeight.SemiBold)
            TaskState(task.operation, task.state, onReview)
        }
        if (task.state is PluginTaskState.Running) {
            CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
        } else {
            TextButton(onClick = { plugins.send(Action.DismissPluginTask(task.id)) }) { Text("关闭") }
        }
    }
}

@Composable
private fun TaskState(operation: PluginOperation, state: PluginTaskState, onReview: () -> Unit) {
    val style = MaterialTheme.typography.bodySmall
    when (state) {
        is PluginTaskState.Running -> Text(operation.runningText, style = style)
        is PluginTaskState.Succeeded ->
            Text(operation.doneText, style = style, color = MaterialTheme.colorScheme.primary)
        is PluginTaskState.Failed -> {
            Text("${operation.failedText}：${state.message}", style = style, color = Red)
            Text(state.reason.nextStep, style = style, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (state.reason == PluginFailure.CHANGED) {
                TextButton(onClick = onReview) { Text("重新查看") }
            }
        }
        is PluginTaskState.Uncertain ->
            Text("结果不确定：${state.message}", style = style, color = MaterialTheme.colorScheme.tertiary)
    }
}

/** 失败的任务里，插件包在确认之后变了的要重新确认。 */
fun reconfirmation(task: PluginTask, plugins: PluginsState): PluginConfirmation? =
    when (task.operation) {
        PluginOperation.INSTALL -> plugins.viewModel.store
            ?.let { store -> (store.official.plugins + store.community.plugins).firstOrNull { it.id == task.id } }
            ?.let { PluginConfirmation.Install(it.source) }
        PluginOperation.UPGRADE -> PluginConfirmation.Upgrade(task.id)
        PluginOperation.UNINSTALL -> null
    }
