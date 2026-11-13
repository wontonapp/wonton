package app.wonton.ui

import android.icu.text.ListFormatter
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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.AddedTools
import app.wonton.protocol.BotRef
import app.wonton.protocol.PluginFailure
import app.wonton.protocol.PluginOperation
import app.wonton.protocol.PluginTask
import app.wonton.protocol.PluginTaskNextStep
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

/** 主屏或商店里打开的详情页、确认页和卸载确认。[open] 打开已安装的插件，[checkInstalled] 回主屏的已安装列表。 */
class PluginNavigation(val open: (PluginEntry) -> Unit, val checkInstalled: () -> Unit) {
    var detail by mutableStateOf<PluginRoute?>(null)
    var confirmation by mutableStateOf<PluginConfirmation?>(null)
    var uninstalling by mutableStateOf<UninstallTarget?>(null)
    var addingSource by mutableStateOf(false)

    /** 正在给哪个插件换版本。 */
    var changingVersion by mutableStateOf<String?>(null)

    /** 详情页上打开的设置页（ADR-0077）。 */
    var settings by mutableStateOf<PluginSettingsTarget?>(null)

    /** 详情页上打开的 Bot 联系人页（ADR-0045、ADR-0050）。 */
    var contacts by mutableStateOf<BotRef?>(null)
}

/** [list] 上叠着打开的详情页和确认页，返回键逐层关掉。 */
@Composable
fun PluginPages(plugins: PluginsState, navigation: PluginNavigation, list: @Composable () -> Unit) {
    val confirmation = navigation.confirmation
    val detail = navigation.detail
    // 商店的一键操作确认页关掉时没装也没升，要告诉核心放下一条排队的（iOS 的 StoreView 同）：
    // 主人自己关（返回、关闭、跳转重置）和核心撤走（装完了、队列空了）都收拢到这一处。
    var presented by remember { mutableStateOf<PluginConfirmation?>(null) }
    LaunchedEffect(confirmation) {
        val previous = presented
        presented = confirmation
        if (previous is PluginConfirmation.Store && confirmation == null) {
            plugins.send(Action.DismissStoreConfirmation)
        }
    }
    val changingVersion = navigation.changingVersion
    val settings = navigation.settings
    val contacts = navigation.contacts
    when {
        contacts != null -> {
            BackHandler { navigation.contacts = null }
            BotContactsPage(plugins.core, contacts, onBack = { navigation.contacts = null })
        }
        settings != null -> {
            BackHandler { navigation.settings = null }
            PluginSettingsScreen(plugins, settings, onClose = { navigation.settings = null })
        }
        confirmation != null -> {
            BackHandler { navigation.confirmation = null }
            InstallConfirmScreen(
                plugins,
                confirmation,
                onClose = { navigation.confirmation = null },
                onCheckInstalled = navigation.checkInstalled,
            )
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
        stringResource(R.string.plugin_uninstall_title, target.title),
        stringResource(R.string.plugin_uninstall_delete_data_warning),
        onDismiss,
        listOf(
            WontonDialogAction(stringResource(R.string.plugin_uninstall_keep_data)) { uninstall(false) },
            WontonDialogAction(stringResource(R.string.plugin_uninstall_delete_data), WontonButtonKind.DANGER) {
                uninstall(true)
            },
            WontonDialogAction(stringResource(R.string.common_cancel), onClick = onDismiss),
        ),
    )
}

/** 安装、升级、卸载的一项进展。插件包在确认之后变了的，[onReview] 重新打开确认页；结果不确定时，[onCheckInstalled] 去已安装列表看。 */
@Composable
fun TaskRow(
    task: PluginTask,
    plugins: PluginsState,
    onReview: () -> Unit,
    onCheckInstalled: () -> Unit,
) {
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        PluginIconImage(task.icon, WontonTheme.size.taskIcon, plugins)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(task.title, style = WontonTheme.typography.itemTitle, color = WontonTheme.colors.textPrimary.toColor())
            TaskState(task, onReview, onCheckInstalled)
            if (task.state is PluginTaskState.Succeeded) task.addedTools?.let { AddedToolsLine(it) }
        }
        if (task.state is PluginTaskState.Running) {
            CircularProgressIndicator(Modifier.size(WontonTheme.size.icon), strokeWidth = SPINNER_STROKE)
        } else {
            WontonIconButton(
                WontonIcon.Close,
                stringResource(R.string.common_close),
                { plugins.send(Action.DismissPluginTask(task.id)) },
            )
        }
    }
}

@Composable
private fun TaskState(task: PluginTask, onReview: () -> Unit, onCheckInstalled: () -> Unit) {
    val colors = WontonTheme.colors
    val style = WontonTheme.typography.supporting
    val operation = task.operation
    when (val state = task.state) {
        is PluginTaskState.Running -> Text(
            if (task.waitingForConnection) {
                stringResource(R.string.plugin_task_waiting_for_connection)
            } else {
                operation.runningText
            },
            style = style,
            color = colors.textSecondary.toColor(),
        )
        is PluginTaskState.Succeeded -> StateLine(WontonIcon.CheckCircleFill, operation.doneText, colors.success)
        is PluginTaskState.Failed -> {
            val problem = state.reason.problem(state.message)
            Text(
                stringResource(
                    when (operation) {
                        PluginOperation.INSTALL -> R.string.plugin_task_install_failed
                        PluginOperation.UPGRADE -> R.string.plugin_task_upgrade_failed
                        PluginOperation.UNINSTALL -> R.string.plugin_task_uninstall_failed
                    },
                    problem.titleText,
                ),
                style = style,
                color = colors.danger.toColor(),
            )
            problem.nextStepText?.let { Text(it, style = style, color = colors.textSecondary.toColor()) }
            ProblemDetail(state.message)
            if (state.reason == PluginFailure.CHANGED) {
                WontonSmallButton(stringResource(R.string.plugin_task_review), onReview)
            }
        }
        is PluginTaskState.Uncertain -> {
            StateLine(WontonIcon.Help, stringResource(R.string.plugin_task_uncertain, state.message), colors.warning)
            if (state.nextStep == PluginTaskNextStep.CHECK_INSTALLED) {
                WontonSmallButton(stringResource(R.string.plugin_task_check_installed), onCheckInstalled)
            }
        }
    }
}

@Composable
private fun AddedToolsLine(added: AddedTools) {
    val colors = WontonTheme.colors
    val style = WontonTheme.typography.supporting
    val listFormat = ListFormatter.getInstance(LocalConfiguration.current.locales[0])
    when (added) {
        is AddedTools.Compared -> if (added.names.isNotEmpty()) {
            Text(
                stringResource(R.string.plugin_task_added_tools, listFormat.format(added.names)),
                style = style,
                color = colors.textSecondary.toColor(),
            )
        }
        is AddedTools.NotCompared -> {
            Text(
                stringResource(R.string.plugin_task_added_tools_unknown),
                style = style,
                color = colors.warning.toColor(),
            )
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
            // 装饰：小图标紧贴说明文字。
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
