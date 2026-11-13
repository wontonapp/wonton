package app.wonton.ui

import android.content.Context
import android.icu.text.ListFormatter
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.LocalUriHandler
import app.wonton.protocol.Action
import app.wonton.protocol.CoreException
import app.wonton.protocol.PluginDetail
import app.wonton.protocol.PluginDetailOutcome
import app.wonton.protocol.PluginImage
import app.wonton.protocol.PluginState
import app.wonton.protocol.PluginTask
import app.wonton.protocol.PluginTaskState
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Blocked
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.ImageBroken
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

/** 截图按手机竖屏的比例占位。 */
private const val SCREENSHOT_RATIO = 9f / 19.5f

/** 插件的详情页：商店里的看来源最新的版本，主屏上的看装着的版本。 */
@Composable
fun PluginDetailScreen(plugins: PluginsState, navigation: PluginNavigation, route: PluginRoute) {
    var attempt by remember { mutableIntStateOf(0) }
    var loaded by remember(route) { mutableStateOf<Result<PluginDetail>?>(null) }
    var toggled by remember(route) { mutableStateOf<PluginDetail?>(null) }
    // 主屏打开的插件先知道 ID：取详情的请求还没回来、任务就做完时，也要看得到“进行中 → 结束”这一步。
    var pluginId by remember(route) { mutableStateOf((route as? PluginRoute.Installed)?.id) }
    LaunchedEffect(route, attempt) {
        loaded = null
        toggled = null
        loaded = try {
            when (val outcome = plugins.core.pluginDetail(route.coreRoute)) {
                is PluginDetailOutcome.Close -> {
                    navigation.detail = null
                    null
                }
                is PluginDetailOutcome.Detail -> {
                    pluginId = outcome.detail.id
                    Result.success(outcome.detail)
                }
            }
        } catch (failure: CoreException) {
            Result.failure(failure)
        }
    }
    val task = plugins.viewModel.tasks.firstOrNull { it.id == pluginId }
    val working = task?.state is PluginTaskState.Running
    var wasWorking by remember { mutableStateOf(false) }
    // 详情只在打开时取一次，任务做完要重取，按钮才对。要不要关这一页由核心判断：插件已经卸掉了，核心让关。
    LaunchedEffect(working) {
        if (wasWorking && !working) {
            try {
                when (val outcome = plugins.core.pluginDetailAfterTask(route.coreRoute)) {
                    is PluginDetailOutcome.Close -> navigation.detail = null
                    is PluginDetailOutcome.Detail -> {
                        toggled = null
                        loaded = Result.success(outcome.detail)
                    }
                }
            } catch (failure: CoreException) {
                loaded = Result.failure(failure)
            }
        }
        wasWorking = working
    }
    WontonScreen(bar = { WontonPushedBar("", onBack = { navigation.detail = null }, showsStatusLine = true) }) {
        val result = loaded
        Box(Modifier.fillMaxSize()) {
            when {
                result == null -> LoadingView()
                result.isFailure -> ProblemView(result.failureProblem, onRetry = { attempt++ })
                else -> {
                    val detail = toggled ?: result.getOrThrow()
                    val installedList = plugins.viewModel.installed
                    val current = if (installedList == null) detail else detail.copy(
                        installed = installedList.plugins.firstOrNull { it.id == detail.id },
                    )
                    DetailContent(current, task, plugins, navigation) { toggled = it }
                }
            }
        }
    }
}

@Composable
private fun DetailContent(
    detail: PluginDetail,
    task: PluginTask?,
    plugins: PluginsState,
    navigation: PluginNavigation,
    onToggled: (PluginDetail) -> Unit,
) {
    val colors = WontonTheme.colors
    // 和 Bot 的已装应用在最上面有一节“资料”（ADR-0044）；家人线的“谁能用”以后加在这一节下面。
    val chat = rememberChat(plugins.core)
    val bot = detail.installed?.let { installed ->
        chat.bots.firstOrNull { it.bot.pluginId == installed.id && it.bot.bot == "default" }
    }
    val scrollState = rememberScrollState()
    val secretsInView = remember { BringIntoViewRequester() }
    // 只有主人能填密钥：家人的“去填写”没有落点，不显示。
    val owner by produceState<Boolean?>(null) { value = runCatching { plugins.core.me().isOwner }.getOrNull() }
    Column(
        Modifier.fillMaxSize().verticalScroll(scrollState)
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        val working = task?.state is PluginTaskState.Running
        if (bot != null) {
            BotProfileSection(bot, plugins)
            BotContactsEntry { navigation.contacts = bot.bot }
        }
        DetailHeader(detail, plugins, detailAction(detail, working, plugins, navigation, LocalContext.current))
        PluginDetailStatus(detail, plugins, working)
        task?.takeIf { it.state !is PluginTaskState.Succeeded || it.hasAddedToolsNews }?.let {
            TaskRow(
                it,
                plugins,
                { navigation.confirmation = reconfirmation(it, plugins) },
                navigation.checkInstalled,
            )
        }
        detail.installed?.notices?.takeIf { it.isNotEmpty() }?.let { NoticeLines(it) }
        GoFillSecrets(detail.installed, owner, secretsInView)
        detail.installed?.let { MyAppsControls(it) }
        if (detail.screenshots.isNotEmpty()) {
            LazyRow(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
                items(detail.screenshots, key = { it.path }) { Screenshot(it, plugins) }
            }
        }
        detail.about?.let { Text(it, style = WontonTheme.typography.body, color = colors.textPrimary.toColor()) }
        detail.installed?.let { InstalledDetailSections(detail, plugins, navigation, onToggled, owner, secretsInView) }
        detail.shares?.let { PluginSharesSection(detail.id, it, plugins, onToggled) }
        DetailSections(detail, plugins, onToggled)
        detail.installed?.takeIf { it.removable && !working }?.let { installed ->
            WontonButton(
                stringResource(R.string.install_change_version),
                { navigation.changingVersion = installed.id },
                Modifier.fillMaxWidth(),
                WontonButtonKind.SECONDARY,
            )
            WontonButton(
                stringResource(R.string.plugin_uninstall_action),
                { navigation.uninstalling = UninstallTarget(installed.id, installed.title) },
                Modifier.fillMaxWidth(),
                WontonButtonKind.DANGER,
            )
        }
    }
}

/** Bot 能存文件的路径（ADR-0027）。撤销只拦后面的调用，已经放进去的文件不动。 */
@Composable
private fun SaveGrants(plugins: PluginsState, id: String, folders: List<String>, onUpdated: (PluginDetail) -> Unit) {
    val colors = WontonTheme.colors
    val scope = rememberCoroutineScope()
    var revoking by remember { mutableStateOf(false) }
    var problem by remember { mutableStateOf<String?>(null) }
    DetailSection(stringResource(R.string.plugin_save_folders)) {
        folders.forEach { folder ->
            Row(
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    folder,
                    Modifier.weight(1f),
                    style = WontonTheme.typography.body,
                    color = colors.textPrimary.toColor(),
                )
                WontonButton(
                    stringResource(R.string.plugin_save_revoke),
                    {
                        revoking = true
                        problem = null
                        scope.launch {
                            try {
                                onUpdated(plugins.core.revokeSaveGrant(id, folder))
                            } catch (failure: CoreException) {
                                problem = failure.toString()
                            }
                            revoking = false
                        }
                    },
                    kind = WontonButtonKind.SECONDARY,
                    enabled = !revoking,
                )
            }
        }
        problem?.let { WontonNotice(WontonIcon.Warning, it) }
    }
}

/** 只有装好的插件才有的节：各页内开关。底栏固定开关在截图之前，位置不在这里。 */
@Composable
private fun InstalledDetailSections(
    detail: PluginDetail,
    plugins: PluginsState,
    navigation: PluginNavigation,
    onToggled: (PluginDetail) -> Unit,
    owner: Boolean?,
    secretsInView: BringIntoViewRequester,
) {
    val installed = detail.installed ?: return
    if (detail.hasSettings) {
        SettingsEntry(detail, navigation)
    }
    Box(Modifier.bringIntoViewRequester(secretsInView)) {
        SecretSection(detail, owner == true, plugins, onToggled)
    }
    detail.lanWebui?.let { InstalledLanWebui(plugins, detail.id, it, onToggled) }
    if (installed.tunnelPorts.isNotEmpty()) PortMappingSection(plugins, installed)
    if (detail.saveGrants.isNotEmpty()) SaveGrants(plugins, detail.id, detail.saveGrants, onToggled)
    ToolPolicySection(plugins, detail.id, detail.version)
    MediaPermissionSection(plugins, detail.id)
    if (installed.writtenBy != null) {
        AskEveryUpdateSection(plugins, detail.id, detail.askEveryUpdate ?: false, onToggled)
    }
}

/** 插件设置（ADR-0077）的入口行，点开整页的原生设置表单。 */
@Composable
private fun SettingsEntry(detail: PluginDetail, navigation: PluginNavigation) {
    DetailSection(stringResource(R.string.plugin_settings_section)) {
        WontonGroup {
            WontonRow(
                stringResource(R.string.plugin_settings_open),
                showsForward = true,
                onClick = { navigation.settings = PluginSettingsTarget(detail.id, detail.title) },
            )
        }
    }
}

private class DetailAction(val title: String, val enabled: Boolean, val perform: () -> Unit)

/** 更新一节：Agent 写的插件有“每次都问”（ADR-0033）。失败显示原因并把开关弹回。 */
@Composable
private fun AskEveryUpdateSection(
    plugins: PluginsState,
    id: String,
    enabled: Boolean,
    onUpdated: (PluginDetail) -> Unit,
) {
    val scope = rememberCoroutineScope()
    var isOn by remember(id) { mutableStateOf(enabled) }
    var busy by remember(id) { mutableStateOf(false) }
    var problem by remember(id) { mutableStateOf<Problem?>(null) }
    DetailSection(stringResource(R.string.plugin_updates_section)) {
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                stringResource(R.string.plugin_ask_every_update),
                Modifier.weight(1f),
                style = WontonTheme.typography.body,
                color = WontonTheme.colors.textPrimary.toColor(),
            )
            WontonSwitch(isOn, { turn ->
                busy = true
                problem = null
                scope.launch {
                    try {
                        val detail = plugins.core.setAskEveryUpdate(id, turn)
                        isOn = detail.askEveryUpdate ?: turn
                        onUpdated(detail)
                    } catch (failure: CoreException) {
                        problem = failure.problem
                        isOn = enabled
                    }
                    busy = false
                }
            }, !busy)
        }
        problem?.let { ProblemNotice(it) }
    }
}

/** 标题旁的按钮：没装的“获取”，能升级的“更新”（和商店列表行尾同一个核心动作，要不要确认由核心判断），装着的“打开”。 */
private fun detailAction(
    detail: PluginDetail, working: Boolean, plugins: PluginsState, navigation: PluginNavigation, context: Context,
): DetailAction? {
    val installed = detail.installed
    val source = detail.source
    val entry = installed?.entries?.firstOrNull()
    // 取预览还没回来时任务还没建，列表行尾靠 pending 转圈，这里用同一个口径。
    val busy = working || (installed != null && installed.id in plugins.viewModel.pending)
    return when {
        installed?.upgrade != null -> DetailAction(
            context.getString(R.string.install_action_upgrade), !busy,
        ) { plugins.send(Action.UpgradeNow(pluginId = installed.id)) }
        installed != null -> entry?.let {
            DetailAction(context.getString(R.string.store_open), !installed.isBusy) { navigation.open(it) }
        }
        source != null -> DetailAction(context.getString(R.string.store_get), detail.blocked == null && !working) {
            navigation.confirmation = PluginConfirmation.Install(source)
        }
        else -> null
    }
}

@Composable
private fun DetailHeader(detail: PluginDetail, plugins: PluginsState, action: DetailAction?) {
    val colors = WontonTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16)) {
            PluginIconImage(detail.icon, WontonTheme.size.detailIcon, plugins)
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                Text(detail.title, style = WontonTheme.typography.sectionTitle, color = colors.textPrimary.toColor())
                Text(detail.summary, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
                Row(
                    horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Tag(detail.trust.label, detail.trust.tint)
                    detail.provider?.let {
                        Text(it, style = WontonTheme.typography.caption, color = colors.textSecondary.toColor())
                    }
                    detail.installed?.writtenBy?.let { writer ->
                        Text(
                            stringResource(R.string.plugin_written_by_agent, writer.title),
                            style = WontonTheme.typography.caption,
                            color = colors.textSecondary.toColor(),
                        )
                    }
                }
            }
        }
        action?.let { WontonButton(it.title, it.perform, Modifier.fillMaxWidth(), enabled = it.enabled) }
        detail.blocked?.let { WontonNotice(WontonIcon.Blocked, it) }
    }
}

/** 一张截图；取不到时画占位（ADR-0017）。 */
@Composable
private fun Screenshot(source: PluginImage, plugins: PluginsState) {
    val colors = WontonTheme.colors
    val image = rememberPluginImage(plugins, source)
    val shape = RoundedCornerShape(WontonTheme.radii.card)
    val height = WontonTheme.size.screenshot
    if (image != null) {
        Image(
            image,
            contentDescription = stringResource(R.string.plugin_screenshot),
            Modifier.height(height).clip(shape),
            contentScale = ContentScale.Fit,
        )
    } else {
        Box(
            Modifier.height(height).aspectRatio(SCREENSHOT_RATIO).background(colors.surfaceSubtle.toColor(), shape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                WontonIcon.ImageBroken,
                contentDescription = stringResource(R.string.plugin_screenshot),
                tint = colors.textSecondary.toColor(),
            )
        }
    }
}

/** 更新说明、权限、要求和其他信息。 */
@Composable
private fun DetailSections(detail: PluginDetail, plugins: PluginsState, onToggled: (PluginDetail) -> Unit) {
    val colors = WontonTheme.colors
    detail.releaseNotes?.let { notes ->
        DetailSection(stringResource(R.string.install_release_notes)) {
            Text(detail.version, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
            Text(notes, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
        }
    }
    PluginPermissionSection(detail, plugins, onToggled)
    if (detail.requirements.isNotEmpty()) {
        DetailSection(stringResource(R.string.plugin_requirements)) {
            detail.requirements.forEach { requirement ->
                Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                    LineIcon(WontonIcon.Check, colors.textSecondary.toColor())
                    Text(requirement.text, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
                }
            }
        }
    }
    DetailSection(stringResource(R.string.plugin_info)) { DetailInfo(detail) }
}

@Composable
private fun DetailInfo(detail: PluginDetail) {
    val uris = LocalUriHandler.current
    val tagList = ListFormatter.getInstance(LocalConfiguration.current.locales[0])
    val info = listOfNotNull(
        stringResource(R.string.settings_version) to detail.version,
        detail.tags.takeIf { it.isNotEmpty() }?.let { stringResource(R.string.plugin_tags) to tagList.format(it) },
        detail.author?.let { stringResource(R.string.plugin_author) to it.name },
        detail.license?.let { stringResource(R.string.plugin_license) to it },
        detail.source?.let { stringResource(R.string.plugin_source) to it },
        detail.stars?.let { stringResource(R.string.plugin_stars) to it.toString() },
    )
    val links = listOfNotNull(
        detail.homepage?.let { stringResource(R.string.plugin_homepage) to it },
        detail.repository?.let { stringResource(R.string.plugin_repository) to it },
    )
    WontonGroup {
        info.forEachIndexed { index, (title, value) ->
            if (index > 0) WontonSeparator()
            WontonRow(title, value)
        }
        links.forEach { (title, link) ->
            WontonSeparator()
            WontonRow(title, showsForward = true, onClick = { uris.openUri(link) })
        }
    }
}
