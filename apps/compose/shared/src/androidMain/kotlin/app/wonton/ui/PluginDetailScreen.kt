package app.wonton.ui

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
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalUriHandler
import app.wonton.protocol.CoreException
import app.wonton.protocol.PluginDetail
import app.wonton.protocol.PluginImage
import app.wonton.protocol.PluginOperation
import app.wonton.protocol.PluginState
import app.wonton.protocol.PluginTask
import app.wonton.protocol.PluginTaskState
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Blocked
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.ImageBroken
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

/** 截图按手机竖屏的比例占位。 */
private const val SCREENSHOT_RATIO = 9f / 19.5f

/** 插件的详情页：商店里的看来源最新的版本，主屏上的看装着的版本。 */
@Composable
fun PluginDetailScreen(plugins: PluginsState, navigation: PluginNavigation, route: PluginRoute) {
    var attempt by remember { mutableIntStateOf(0) }
    var pluginId by remember(route) { mutableStateOf<String?>(null) }
    var toggled by remember(route) { mutableStateOf<PluginDetail?>(null) }
    val loaded by produceState<Result<PluginDetail>?>(null, route, attempt) {
        value = null
        toggled = null
        value = try {
            Result.success(
                when (route) {
                    is PluginRoute.Store -> plugins.core.storePlugin(route.source)
                    is PluginRoute.Installed -> plugins.core.installedPlugin(route.id)
                }.also { pluginId = it.id },
            )
        } catch (failure: CoreException) {
            Result.failure(failure)
        }
    }
    val task = plugins.viewModel.tasks.firstOrNull { it.id == pluginId }
    val working = task?.state is PluginTaskState.Running
    var wasWorking by remember { mutableStateOf(false) }
    // 详情只在打开时取一次，任务做完要重取，按钮才对。主屏上打开的插件卸载以后，这一页没有东西可看了。
    LaunchedEffect(working) {
        if (wasWorking && !working) {
            val removed = task?.operation == PluginOperation.UNINSTALL && task.state is PluginTaskState.Succeeded
            if (route is PluginRoute.Installed && removed) navigation.detail = null else attempt++
        }
        wasWorking = working
    }
    WontonScreen(bar = { WontonPushedBar("", onBack = { navigation.detail = null }) }) {
        val result = loaded
        Box(Modifier.fillMaxSize()) {
            when {
                result == null -> LoadingView()
                result.isFailure -> ProblemView(result.failureProblem, onRetry = { attempt++ })
                else -> DetailContent(toggled ?: result.getOrThrow(), task, plugins, navigation) { toggled = it }
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
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        val working = task?.state is PluginTaskState.Running
        DetailHeader(detail, plugins, detailAction(detail, working, navigation))
        task?.takeIf { it.state !is PluginTaskState.Succeeded }?.let {
            TaskRow(it, plugins) { navigation.confirmation = reconfirmation(it, plugins) }
        }
        detail.installed?.notices?.takeIf { it.isNotEmpty() }?.let { NoticeLines(it) }
        if (detail.screenshots.isNotEmpty()) {
            LazyRow(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
                items(detail.screenshots, key = { it.path }) { Screenshot(it, plugins) }
            }
        }
        detail.about?.let { Text(it, style = WontonTheme.typography.body, color = colors.textPrimary.toColor()) }
        val offer = detail.lanWebui
        if (offer != null && detail.installed != null) InstalledLanWebui(plugins, detail.id, offer, onToggled)
        DetailSections(detail)
        detail.installed?.takeIf { it.removable && !working }?.let { installed ->
            WontonButton(
                "卸载",
                { navigation.uninstalling = UninstallTarget(installed.id, installed.title) },
                Modifier.fillMaxWidth(),
                WontonButtonKind.DANGER,
            )
        }
    }
}

private class DetailAction(val title: String, val enabled: Boolean, val perform: () -> Unit)

/** 标题旁的按钮：没装的“获取”，能升级的“更新”，装着的“打开”。 */
private fun detailAction(detail: PluginDetail, working: Boolean, navigation: PluginNavigation): DetailAction? {
    val installed = detail.installed
    val source = detail.source
    val entry = installed?.entries?.firstOrNull()
    return when {
        installed?.upgrade != null -> DetailAction("更新", !installed.isBusy) {
            navigation.confirmation = PluginConfirmation.Upgrade(installed.id)
        }
        installed != null -> entry?.let { DetailAction("打开", !installed.isBusy) { navigation.open(it) } }
        source != null -> DetailAction("获取", detail.blocked == null && !working) {
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
                Tag(detail.trust.label, detail.trust.tint)
            }
        }
        action?.let { WontonButton(it.title, it.perform, Modifier.fillMaxWidth(), enabled = it.enabled) }
        detail.blocked?.let { WontonNotice(WontonIcon.Blocked, it) }
        (detail.installed?.status as? PluginState.Degraded)?.let { WontonNotice(WontonIcon.Warning, it.reason) }
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
        Image(image, contentDescription = null, Modifier.height(height).clip(shape), contentScale = ContentScale.Fit)
    } else {
        Box(
            Modifier.height(height).aspectRatio(SCREENSHOT_RATIO).background(colors.surfaceSubtle.toColor(), shape),
            contentAlignment = Alignment.Center,
        ) {
            Icon(WontonIcon.ImageBroken, contentDescription = null, tint = colors.textSecondary.toColor())
        }
    }
}

/** 更新说明、权限、要求和其他信息。 */
@Composable
private fun DetailSections(detail: PluginDetail) {
    val colors = WontonTheme.colors
    detail.releaseNotes?.let { notes ->
        DetailSection("新内容") {
            Text(detail.version, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
            Text(notes, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
        }
    }
    if (detail.permissions.isNotEmpty()) {
        DetailSection("权限") { detail.permissions.forEach { PermissionRow(it) } }
    }
    if (detail.requirements.isNotEmpty()) {
        DetailSection("要求") {
            detail.requirements.forEach { requirement ->
                Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                    LineIcon(WontonIcon.Check, colors.textSecondary.toColor())
                    Text(requirement.text, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
                }
            }
        }
    }
    DetailSection("信息") { DetailInfo(detail) }
}

@Composable
private fun DetailInfo(detail: PluginDetail) {
    val uris = LocalUriHandler.current
    val info = listOfNotNull(
        "版本" to detail.version,
        detail.tags.takeIf { it.isNotEmpty() }?.let { "标签" to it.joinToString("、") },
        detail.author?.let { "作者" to it.name },
        detail.license?.let { "许可证" to it },
        detail.source?.let { "来源" to it },
        detail.stars?.let { "星标" to it.toString() },
    )
    val links = listOfNotNull(detail.homepage?.let { "主页" to it }, detail.repository?.let { "代码仓库" to it })
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
