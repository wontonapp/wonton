package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.AddedTools
import app.wonton.protocol.PluginFailure
import app.wonton.protocol.PluginOperation
import app.wonton.protocol.PluginTask
import app.wonton.protocol.PluginTaskState
import app.wonton.protocol.Rgb
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.CheckCircleFill
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Help
import app.wonton.ui.icons.WontonIcon

private val SPINNER_STROKE = 2.dp

/** 说明行高 20、小图标 14，差的一半。 */
private val STATE_ICON_OFFSET = 3.dp

/** 主屏或商店里打开的详情页、确认页和卸载确认。[open] 打开已安装的插件。 */
class PluginNavigation(val open: (PluginEntry) -> Unit) {
    var detail by mutableStateOf<PluginRoute?>(null)
    var confirmation by mutableStateOf<PluginConfirmation?>(null)
    var uninstalling by mutableStateOf<UninstallTarget?>(null)
    var addingSource by mutableStateOf(false)

    /** 正在给哪个插件换版本。 */
    var changingVersion by mutableStateOf<String?>(null)
}

/** [list] 上叠着打开的详情页和确认页，返回键逐层关掉。 */
@Composable
fun PluginPages(plugins: PluginsState, navigation: PluginNavigation, list: @Composable () -> Unit) {
    val confirmation = navigation.confirmation
    val detail = navigation.detail
    val changingVersion = navigation.changingVersion
    when {
        confirmation != null -> {
            BackHandler { navigation.confirmation = null }
            InstallConfirmScreen(plugins, confirmation, onClose = { navigation.confirmation = null })
        }
        navigation.addingSource -> {
            BackHandler { navigation.addingSource = false }
            AddSourceScreen(plugins, navigation)
        }
        changingVersion != null -> {
            BackHandler { navigation.changingVersion = null }
            ChangeVersionScreen(plugins, navigation, changingVersion)
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
internal fun UninstallDialog(target: UninstallTarget, plugins: PluginsState, onDismiss: () -> Unit) {
    val uninstall = { deleteData: Boolean ->
        onDismiss()
        plugins.send(Action.UninstallPlugin(target.id, deleteData))
    }
    WontonDialog(
        "卸载“${target.title}”？",
        "删除数据会一并删掉它的密钥和 Bot 账号，不能恢复。",
        onDismiss,
        listOf(
            WontonDialogAction("卸载，保留数据") { uninstall(false) },
            WontonDialogAction("卸载并删除数据", WontonButtonKind.DANGER) { uninstall(true) },
            WontonDialogAction("取消", onClick = onDismiss),
        ),
    )
}

/** 安装、升级、卸载的一项进展。插件包在确认之后变了的，[onReview] 重新打开确认页。 */
@Composable
fun TaskRow(task: PluginTask, plugins: PluginsState, onReview: () -> Unit) {
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        PluginIconImage(task.icon, WontonTheme.size.taskIcon, plugins)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(task.title, style = WontonTheme.typography.itemTitle, color = WontonTheme.colors.textPrimary.toColor())
            TaskState(task.operation, task.state, onReview)
            if (task.state is PluginTaskState.Succeeded) task.addedTools?.let { AddedToolsLine(it) }
        }
        if (task.state is PluginTaskState.Running) {
            CircularProgressIndicator(Modifier.size(WontonTheme.size.icon), strokeWidth = SPINNER_STROKE)
        } else {
            WontonIconButton(WontonIcon.Close, "关闭", { plugins.send(Action.DismissPluginTask(task.id)) })
        }
    }
}

@Composable
private fun TaskState(operation: PluginOperation, state: PluginTaskState, onReview: () -> Unit) {
    val colors = WontonTheme.colors
    val style = WontonTheme.typography.supporting
    when (state) {
        is PluginTaskState.Running -> Text(operation.runningText, style = style, color = colors.textSecondary.toColor())
        is PluginTaskState.Succeeded -> StateLine(WontonIcon.CheckCircleFill, operation.doneText, colors.success)
        is PluginTaskState.Failed -> {
            val problem = state.reason.problem(state.message)
            Text("${operation.failedText}：${problem.title}", style = style, color = colors.danger.toColor())
            problem.nextStep?.let { Text(it, style = style, color = colors.textSecondary.toColor()) }
            ProblemDetail(state.message)
            if (state.reason == PluginFailure.CHANGED) WontonSmallButton("重新查看", onReview)
        }
        is PluginTaskState.Uncertain -> StateLine(WontonIcon.Help, "结果不确定：${state.message}", colors.warning)
    }
}

@Composable
private fun AddedToolsLine(added: AddedTools) {
    val colors = WontonTheme.colors
    val style = WontonTheme.typography.supporting
    when (added) {
        is AddedTools.Compared -> if (added.names.isNotEmpty()) {
            Text("新增的工具：${added.names.joinToString("、")}", style = style, color = colors.textSecondary.toColor())
        }
        is AddedTools.NotCompared -> {
            Text("不知道这次多了哪些工具", style = style, color = colors.warning.toColor())
            ProblemDetail(added.reason)
        }
    }
}

/** 详情页重取后已经看得出升级成功，但多了哪些工具只有这一行会说。 */
val PluginTask.hasAddedToolsNews: Boolean
    get() = when (val added = addedTools) {
        is AddedTools.Compared -> added.names.isNotEmpty()
        is AddedTools.NotCompared -> true
        null -> false
    }

/** 一行结果：小图标紧贴文字，和文字的第一行对齐。 */
@Composable
fun StateLine(icon: ImageVector, text: String, tint: Rgb) {
    Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        Icon(
            icon,
            contentDescription = null,
            modifier = Modifier.padding(top = STATE_ICON_OFFSET).size(WontonTheme.size.iconTiny),
            tint = tint.toColor(),
        )
        Text(text, style = WontonTheme.typography.supporting, color = tint.toColor())
    }
}

/** 失败的任务里，插件包在确认之后变了的要重新确认。 */
fun reconfirmation(task: PluginTask, plugins: PluginsState): PluginConfirmation? =
    when (task.operation) {
        PluginOperation.INSTALL -> plugins.viewModel.store
            ?.let { store -> store.allPlugins.firstOrNull { it.id == task.id } }
            ?.let { PluginConfirmation.Install(it.source) }
        PluginOperation.UPGRADE -> PluginConfirmation.Upgrade(task.id)
        PluginOperation.UNINSTALL -> null
    }
