package app.wonton.ui

import app.wonton.client.PluginInstallAction
import app.wonton.client.previewInstallAction
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
import androidx.compose.material3.LinearProgressIndicator
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
import app.wonton.protocol.PluginFailure
import app.wonton.protocol.PluginOperation
import app.wonton.protocol.PluginTask
import app.wonton.protocol.PluginTaskNextStep
import app.wonton.protocol.PluginTaskState
import app.wonton.ui.design.wontonContentPadding
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
    onCheckInstalled: () -> Unit,
) {
    val isUpgrade = confirmation.isUpgrade
    var attempt by remember { mutableIntStateOf(0) }
    var rejection by remember { mutableStateOf<Problem?>(null) }
    var lanWebui by remember { mutableStateOf(false) }
    var enabled by remember { mutableStateOf<Set<String>>(emptySet()) }
    var secrets by remember(confirmation) { mutableStateOf<Map<String, String>>(emptyMap()) }
    var audience by remember(confirmation) { mutableStateOf<app.wonton.protocol.PluginAudienceView?>(null) }
    var fillingProfile by remember { mutableStateOf(false) }
    var submittedId by remember(confirmation) { mutableStateOf<String?>(null) }
    var previousTask by remember(confirmation) { mutableStateOf<PluginTask?>(null) }
    val loaded by produceState<Result<InstallPreview>?>(null, confirmation, attempt) {
        value = null
        value = try {
            Result.success(
                loadInstallPreview(plugins, confirmation).also { preview ->
                    lanWebui = preview.lanWebui?.enabled ?: false
                    enabled = preview.permissions.filter { it.optional && it.enabled }.map { it.permission }.toSet()
                    audience = preview.audience
                },
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
        InstallCompletion(plugins, id, previousTask, onClose, onCheckInstalled) {
            submittedId = null
            attempt++
        }
        return
    }
    SecureScreen()
    val title = confirmation.actionTitle
    WontonScreen(bar = { WontonPushedBar(title, onClose, WontonIcon.Close, stringResource(R.string.common_close)) }) {
        val result = loaded
        val preview = result?.getOrNull()
        Column(Modifier.fillMaxSize()) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                PreviewResult(
                    result,
                    onRetry = { attempt++ },
                    preview = preview,
                    plugins = plugins,
                    lanWebui = lanWebui,
                    enabled = enabled,
                    onFillProfile = { fillingProfile = true },
                    onLanWebui = { lanWebui = it },
                    onOptional = { permission, on ->
                        enabled = if (on) enabled + permission else enabled - permission
                    },
                    secrets = secrets,
                    onSecret = { name, value -> secrets = secrets + (name to value) },
                    audience = audience,
                    onAudience = { audience = it },
                )
            }
            preview?.let { current ->
                val chosen = current.withPermissionChoices(lanWebui, enabled).copy(audience = audience)
                val sent = secretValues(chosen.secretInputs, secrets)
                ConfirmInstallButton(
                    chosen,
                    if (previewInstallAction(chosen) == PluginInstallAction.REQUEST_ADMIN) {
                        stringResource(R.string.install_request_admin)
                    } else { title },
                    secretsReady = !missingRequiredSecret(chosen.secretInputs, secrets),
                ) {
                    val action = if (isUpgrade) {
                        Action.UpgradePlugin(chosen, sent, emptyMap())
                    } else {
                        Action.InstallPlugin(chosen, sent, emptyMap())
                    }
                    previousTask = plugins.viewModel.tasks.firstOrNull { it.id == chosen.id }
                    rejection = plugins.core.send(action)
                    if (rejection == null) {
                        when {
                            onAccepted != null -> onAccepted()
                            isUpgrade -> onClose()
                            else -> submittedId = chosen.id
                        }
                    }
                }
            }
        }
    }
    rejection?.let { ProblemDialog(it) { rejection = null } }
}

private suspend fun loadInstallPreview(plugins: PluginsState, confirmation: PluginConfirmation): InstallPreview =
    // 核心取好预览请主人拿主意的一键操作（ADR-0017）：不再重新取。
    confirmation.preloaded ?: when (confirmation) {
        is PluginConfirmation.Install -> plugins.core.previewInstall(confirmation.source)
        is PluginConfirmation.Upgrade -> plugins.core.previewUpgrade(confirmation.id)
        is PluginConfirmation.Upload -> plugins.core.previewUpload(confirmation.archive)
        is PluginConfirmation.UpgradeFrom -> plugins.core.previewUpgradeFrom(confirmation.id, confirmation.source)
        is PluginConfirmation.UpgradeUpload -> plugins.core.previewUpgradeUpload(confirmation.id, confirmation.archive)
        is PluginConfirmation.Store -> error("core preview already returned above")
    }

/** 装载、失败和可预览三种状态里画哪一种。 */
@Composable
private fun PreviewResult(
    result: Result<InstallPreview>?,
    onRetry: () -> Unit,
    preview: InstallPreview?,
    plugins: PluginsState,
    lanWebui: Boolean,
    enabled: Set<String>,
    onFillProfile: () -> Unit,
    onLanWebui: (Boolean) -> Unit,
    onOptional: (String, Boolean) -> Unit,
    secrets: Map<String, String>,
    onSecret: (String, String) -> Unit,
    audience: app.wonton.protocol.PluginAudienceView?,
    onAudience: (app.wonton.protocol.PluginAudienceView) -> Unit,
) {
    when {
        result == null -> LoadingView()
        result.isFailure -> ProblemView(result.failureProblem, onRetry = onRetry)
        preview != null -> PreviewContent(
            preview,
            plugins,
            lanWebui,
            enabled,
            onFillProfile = onFillProfile,
            onLanWebui = onLanWebui,
            onOptional = onOptional,
            secrets = secrets,
            onSecret = onSecret,
            audience = audience,
            onAudience = onAudience,
        )
    }
}

/** 留在安装页等核心任务完成，成功后才提供固定入口。 */
@Composable
private fun InstallCompletion(
    plugins: PluginsState,
    id: String,
    previousTask: PluginTask?,
    onClose: () -> Unit,
    onCheckInstalled: () -> Unit,
    onReview: () -> Unit,
) {
    val task = plugins.viewModel.tasks.firstOrNull { it.id == id }?.takeUnless { it == previousTask }
    val installed = plugins.viewModel.installed?.plugins?.firstOrNull { it.id == id }
    val succeeded = task?.state is PluginTaskState.Succeeded
    WontonScreen(
        bar = {
            WontonPushedBar(
                stringResource(R.string.install_completion_title),
                onClose,
                showsStatus = true,
            )
        },
    ) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            when (val state = task?.state) {
                null, is PluginTaskState.Running -> {
                    CircularProgressIndicator()
                    Text(
                        stringResource(
                            if (task?.waitingForConnection == true) {
                                R.string.plugin_task_waiting_for_connection
                            } else {
                                R.string.install_completion_installing
                            },
                        ),
                        style = WontonTheme.typography.body,
                        color = WontonTheme.colors.textPrimary.toColor(),
                    )
                    task?.installProgress?.let { progress ->
                        Text(
                            stageText(progress.stage, PluginOperation.INSTALL),
                            style = WontonTheme.typography.supporting,
                            color = WontonTheme.colors.textSecondary.toColor(),
                        )
                        // 家里的服务器算得出比例的只有拉镜像这一步，其余阶段照实说，不造百分比。
                        progress.percent?.let { LinearProgressIndicator(progress = { it.toFloat() / 100f }) }
                    }
                }
                is PluginTaskState.Succeeded -> Text(stringResource(R.string.install_completion_done),
                    style = WontonTheme.typography.itemTitle, color = WontonTheme.colors.textPrimary.toColor())
                is PluginTaskState.Failed -> InstallFailure(state.reason, state.message, onReview)
                is PluginTaskState.Uncertain -> {
                    ProblemNotice(
                        Problem(
                            stringResource(R.string.install_completion_uncertain),
                            state.nextStep.takeIf { it == PluginTaskNextStep.CHECK_INSTALLED }
                                ?.let { stringResource(R.string.plugin_task_check_installed_next_step) },
                            state.message,
                        ),
                    )
                    if (state.nextStep == PluginTaskNextStep.CHECK_INSTALLED) {
                        WontonButton(stringResource(R.string.plugin_task_check_installed), onCheckInstalled)
                    }
                }
            }
            plugins.viewModel.problem?.let { ProblemNotice(it.problem) }
            if (succeeded && installed != null) MyAppAddButton(installed)
            if (succeeded) OnboardingTipNotice(app.wonton.protocol.OnboardingTip.APP_INSTALLED)
            WontonButton(stringResource(R.string.install_done), onClose)
        }
    }
}

/** 安装失败：标题和下一步按类别出，标题沿用商店任务行的“安装失败：原因”格式。 */
@Composable
fun InstallFailure(reason: PluginFailure, message: String, onReview: () -> Unit) {
    val problem = reason.problem(message)
    ProblemNotice(Problem(
        title = stringResource(R.string.plugin_task_install_failed, problem.titleText),
        nextStep = problem.nextStepText,
        detail = problem.detail,
    ))
    WontonButton(stringResource(R.string.plugin_task_review), onReview)
}

@Composable
private fun PreviewContent(
    preview: InstallPreview,
    plugins: PluginsState,
    lanWebui: Boolean,
    enabled: Set<String>,
    onFillProfile: () -> Unit,
    onLanWebui: (Boolean) -> Unit,
    onOptional: (String, Boolean) -> Unit,
    secrets: Map<String, String>,
    onSecret: (String, String) -> Unit,
    audience: app.wonton.protocol.PluginAudienceView?,
    onAudience: (app.wonton.protocol.PluginAudienceView) -> Unit,
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
        if (preview.notices.isNotEmpty()) NoticeLines(preview.notices)
        audience?.let { PluginAudienceSection(it, onChange = onAudience) }
        InstallPermissionChoices(preview.permissions, enabled, onOptional)
        if (preview.secretInputs.isNotEmpty()) {
            DetailSection(stringResource(R.string.secret_section_title)) {
                SecretInputsSection(preview.secretInputs, secrets, onChange = onSecret)
            }
        }
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
