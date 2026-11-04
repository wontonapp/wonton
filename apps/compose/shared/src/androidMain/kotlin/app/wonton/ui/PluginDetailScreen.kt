package app.wonton.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import app.wonton.protocol.CoreException
import app.wonton.protocol.PluginDetail
import app.wonton.protocol.PluginImage
import app.wonton.protocol.PluginOperation
import app.wonton.protocol.PluginState
import app.wonton.protocol.PluginTask
import app.wonton.protocol.PluginTaskState

/** 截图按手机竖屏的比例占位。 */
private const val SCREENSHOT_RATIO = 9f / 19.5f

/** 插件的详情页：商店里的看来源最新的版本，主屏上的看装着的版本。 */
@OptIn(ExperimentalMaterial3Api::class)
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
    Scaffold(
        topBar = {
            TopAppBar(
                title = {},
                navigationIcon = { TextButton(onClick = { navigation.detail = null }) { Text("返回") } },
            )
        },
    ) { padding ->
        val result = loaded
        Box(Modifier.padding(padding).fillMaxSize()) {
            when {
                result == null -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                result.isFailure -> ProblemView(result.exceptionOrNull().toString(), null, onRetry = { attempt++ })
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
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp),
    ) {
        val working = task?.state is PluginTaskState.Running
        DetailHeader(detail, plugins, detailAction(detail, working, navigation))
        task?.takeIf { it.state !is PluginTaskState.Succeeded }?.let {
            TaskRow(it, plugins) { navigation.confirmation = reconfirmation(it, plugins) }
        }
        detail.installed?.notices?.takeIf { it.isNotEmpty() }?.let { NoticeLines(it) }
        if (detail.screenshots.isNotEmpty()) {
            LazyRow(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                items(detail.screenshots, key = { it.path }) { Screenshot(it, plugins) }
            }
        }
        detail.about?.let { Text(it) }
        val offer = detail.lanWebui
        if (offer != null && detail.installed != null) InstalledLanWebui(plugins, detail.id, offer, onToggled)
        DetailSections(detail)
        detail.installed?.takeIf { it.removable && !working }?.let { installed ->
            TextButton(onClick = { navigation.uninstalling = UninstallTarget(installed.id, installed.title) }) {
                Text("卸载", color = Red)
            }
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
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
            PluginIconImage(detail.icon, detail.title, 112.dp, plugins)
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(detail.title, style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Bold)
                Text(detail.summary, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Tag(detail.trust.label, detail.trust.tint)
                action?.let { Button(onClick = it.perform, enabled = it.enabled) { Text(it.title) } }
            }
        }
        detail.blocked?.let { Text(it, color = Red) }
        (detail.installed?.status as? PluginState.Degraded)?.let { Text(it.reason, color = Red) }
    }
}

/** 一张截图；取不到时画占位（ADR-0017）。 */
@Composable
private fun Screenshot(source: PluginImage, plugins: PluginsState) {
    val image = rememberPluginImage(plugins, source)
    val shape = RoundedCornerShape(18.dp)
    if (image != null) {
        Image(image, contentDescription = null, Modifier.height(380.dp).clip(shape), contentScale = ContentScale.Fit)
    } else {
        Box(
            Modifier.height(380.dp).aspectRatio(SCREENSHOT_RATIO)
                .background(MaterialTheme.colorScheme.surfaceVariant, shape),
        )
    }
}

/** 更新说明、权限、要求和其他信息。 */
@Composable
private fun DetailSections(detail: PluginDetail) {
    detail.releaseNotes?.let { notes ->
        DetailSection("新内容") {
            Text(detail.version, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(notes)
        }
    }
    if (detail.permissions.isNotEmpty()) {
        DetailSection("权限") { detail.permissions.forEach { PermissionRow(it) } }
    }
    if (detail.requirements.isNotEmpty()) {
        DetailSection("要求") { detail.requirements.forEach { Text(it) } }
    }
    val uris = LocalUriHandler.current
    DetailSection("信息") {
        InfoRow("版本", detail.version)
        detail.category?.let { InfoRow("分类", it.label) }
        detail.author?.let { InfoRow("作者", it.name) }
        detail.license?.let { InfoRow("许可证", it) }
        detail.source?.let { InfoRow("来源", it) }
        detail.stars?.let { InfoRow("星标", it.toString()) }
        detail.homepage?.let { link -> TextButton(onClick = { uris.openUri(link) }) { Text("主页") } }
        detail.repository?.let { link -> TextButton(onClick = { uris.openUri(link) }) { Text("代码仓库") } }
    }
}

@Composable
private fun DetailSection(title: String, content: @Composable () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(title, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold)
        content()
    }
}

@Composable
private fun InfoRow(label: String, value: String) {
    Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        Text(label, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(value, Modifier.weight(1f), textAlign = TextAlign.End)
    }
}
