package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.CoreException
import app.wonton.protocol.InstallPreview
import app.wonton.protocol.PluginTask
import app.wonton.protocol.PluginTaskState
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.LocalWontonBottomInset
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Blocked
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

/** 安装、升级的确认页：原样显示家里的服务器给的预览，确认后把这份预览交回核心，授予的就是这里列出的权限。 */
@Composable
fun InstallConfirmScreen(
    plugins: PluginsState,
    confirmation: PluginConfirmation,
    onAccepted: (() -> Unit)? = null,
    onClose: () -> Unit,
) {
    val isUpgrade = confirmation.isUpgrade
    var attempt by remember { mutableIntStateOf(0) }
    var rejection by remember { mutableStateOf<Problem?>(null) }
    var lanWebui by remember { mutableStateOf(false) }
    var fillingProfile by remember { mutableStateOf(false) }
    var submittedId by remember(confirmation) { mutableStateOf<String?>(null) }
    var previousTask by remember(confirmation) { mutableStateOf<PluginTask?>(null) }
    val loaded by produceState<Result<InstallPreview>?>(null, confirmation, attempt) {
        value = null
        value = try {
            Result.success(
                loadInstallPreview(plugins, confirmation).also { lanWebui = it.lanWebui?.enabled ?: false },
            )
        } catch (failure: CoreException) {
            Result.failure(failure)
        }
    }
    if (fillingProfile) {
        // 填完回到这里，预览重新算，缺的资料有了就能装。
        OwnerProfilePage(plugins) {
            fillingProfile = false
            attempt++
        }
        return
    }
    submittedId?.let { id ->
        InstallCompletion(plugins, id, previousTask, onClose) {
            submittedId = null
            attempt++
        }
        return
    }
    val title = confirmation.actionTitle
    WontonScreen(bar = { WontonPushedBar(title, onClose, WontonIcon.Close, stringResource(R.string.common_close)) }) {
        val result = loaded
        val preview = result?.getOrNull()
        Column(Modifier.fillMaxSize()) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                when {
                    result == null -> LoadingView()
                    result.isFailure -> ProblemView(result.failureProblem, onRetry = { attempt++ })
                    preview != null -> PreviewContent(
                        preview,
                        plugins,
                        lanWebui,
                        onFillProfile = { fillingProfile = true },
                        onLanWebui = { lanWebui = it },
                    )
                }
            }
            preview?.let {
                WontonButton(
                    title,
                    {
                        // 在这里直接发给核心：被拒绝时原因留在确认页上，不随确认页关掉。
                        val shown = it.copy(lanWebui = it.lanWebui?.copy(enabled = lanWebui))
                        val action = if (isUpgrade) Action.UpgradePlugin(shown) else Action.InstallPlugin(shown)
                        previousTask = plugins.viewModel.tasks.firstOrNull { task -> task.id == shown.id }
                        rejection = plugins.core.send(action)
                        if (rejection == null) {
                            when {
                                onAccepted != null -> onAccepted()
                                isUpgrade -> onClose()
                                else -> submittedId = shown.id
                            }
                        }
                    },
                    Modifier.padding(bottom = LocalWontonBottomInset.current)
                        .padding(WontonTheme.spacing.pageInset)
                        .fillMaxWidth(),
                    // 装不了（blocked）、缺资料或者有核心不支持的权限时不能确认。
                    enabled = it.blocked == null && it.missingProfile == null &&
                        it.permissions.all { line -> line.supported },
                )
            }
        }
    }
    rejection?.let { ProblemDialog(it) { rejection = null } }
}

private suspend fun loadInstallPreview(plugins: PluginsState, confirmation: PluginConfirmation): InstallPreview =
    when (confirmation) {
        is PluginConfirmation.Install -> plugins.core.previewInstall(confirmation.source)
        is PluginConfirmation.Upgrade -> plugins.core.previewUpgrade(confirmation.id)
        is PluginConfirmation.Upload -> plugins.core.previewUpload(confirmation.archive)
        is PluginConfirmation.UpgradeFrom -> plugins.core.previewUpgradeFrom(confirmation.id, confirmation.source)
        is PluginConfirmation.UpgradeUpload -> plugins.core.previewUpgradeUpload(confirmation.id, confirmation.archive)
    }

/** 留在安装页等核心任务完成，成功后才提供固定入口。 */
@Composable
private fun InstallCompletion(
    plugins: PluginsState,
    id: String,
    previousTask: PluginTask?,
    onClose: () -> Unit,
    onReview: () -> Unit,
) {
    val task = plugins.viewModel.tasks.firstOrNull { it.id == id }?.takeUnless { it == previousTask }
    val installed = plugins.viewModel.installed?.plugins?.firstOrNull { it.id == id }
    val succeeded = task?.state is PluginTaskState.Succeeded
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.install_completion_title), onClose) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            when (val state = task?.state) {
                null, is PluginTaskState.Running -> {
                    CircularProgressIndicator()
                    Text(stringResource(R.string.install_completion_installing), style = WontonTheme.typography.body,
                        color = WontonTheme.colors.textPrimary.toColor())
                }
                is PluginTaskState.Succeeded -> Text(stringResource(R.string.install_completion_done),
                    style = WontonTheme.typography.itemTitle, color = WontonTheme.colors.textPrimary.toColor())
                is PluginTaskState.Failed -> {
                    ProblemNotice(Problem(stringResource(R.string.install_completion_failed), null, state.message))
                    WontonButton(stringResource(R.string.plugin_task_review), onReview)
                }
                is PluginTaskState.Uncertain -> ProblemNotice(Problem(
                    stringResource(R.string.install_completion_uncertain), null, state.message))
            }
            plugins.viewModel.problem?.let { ProblemNotice(it.problem) }
            if (succeeded && installed != null) PluginTabPinButton(installed)
            if (succeeded) OnboardingTipNotice(app.wonton.protocol.OnboardingTip.APP_INSTALLED)
            WontonButton(stringResource(R.string.install_done), onClose)
        }
    }
}

@Composable
private fun PreviewContent(
    preview: InstallPreview,
    plugins: PluginsState,
    lanWebui: Boolean,
    onFillProfile: () -> Unit,
    onLanWebui: (Boolean) -> Unit,
) {
    val colors = WontonTheme.colors
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            PluginIconImage(preview.icon, WontonTheme.size.previewIcon, plugins)
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                Text(preview.title, style = WontonTheme.typography.itemTitle, color = colors.textPrimary.toColor())
                val version = preview.upgradeFrom?.let { "$it → ${preview.version}" } ?: preview.version
                Text(version, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
                Row(
                    horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Tag(preview.trust.label, preview.trust.tint)
                    // 提供方是核实过的，有它就不写清单里自称的作者；两个都放一行放不下。
                    (preview.provider ?: preview.author?.name)?.let {
                        Text(it, style = WontonTheme.typography.caption, color = colors.textSecondary.toColor())
                    }
                }
            }
        }
        preview.blocked?.let { WontonNotice(WontonIcon.Blocked, it) }
        preview.missingProfile?.let {
            WontonNotice(WontonIcon.Info, it, isError = false) {
                WontonButton(stringResource(R.string.install_fill_profile), onFillProfile)
            }
        }
        InstallAccessDetails(preview.notices, preview.permissions)
        AlwaysAllowed(preview, plugins)
        preview.lanWebui?.let { LanWebuiToggle(it, lanWebui, enabled = true, onLanWebui) }
        preview.releaseNotes?.let { notes ->
            DetailSection(stringResource(R.string.install_release_notes)) {
                Text(notes, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
            }
        }
    }
}

/** 升级时已经始终允许的 Bot，当场可以关。关掉是立刻生效的，主人不确认升级也一样。 */
@Composable
private fun AlwaysAllowed(preview: InstallPreview, plugins: PluginsState) {
    var bots by remember(preview) { mutableStateOf(preview.alwaysAllowed) }
    var problem by remember(preview) { mutableStateOf<Problem?>(null) }
    val scope = rememberCoroutineScope()
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        BotSwitches(
            stringResource(R.string.tool_policy_always_title),
            stringResource(R.string.install_always_allowed_risk),
            bots,
            enabled = true,
        ) { caller, isOn ->
            scope.launch {
                try {
                    plugins.core.setToolAlways(preview.id, caller, isOn)
                    problem = null
                    bots = bots.map { if (it.caller == caller) it.copy(enabled = isOn) else it }
                } catch (failure: CoreException) {
                    problem = failure.problem
                }
            }
        }
        problem?.let { ProblemNotice(it) }
    }
}
