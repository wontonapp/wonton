package app.wonton.ui

import android.icu.text.CompactDecimalFormat
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.AddedSource
import app.wonton.protocol.PluginTaskState
import app.wonton.protocol.PluginTrust
import app.wonton.protocol.SecurityTier
import app.wonton.protocol.StoreItem
import app.wonton.protocol.StoreListing
import app.wonton.protocol.StoreConfirmation
import app.wonton.protocol.UpdatablePlugin
import app.wonton.protocol.StoreSection
import app.wonton.ui.design.wontonContentPaddingValues
import app.wonton.ui.design.WontonChip
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSearchField
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.StarFill
import app.wonton.ui.icons.WontonIcon
import java.util.Locale

private val ROW_GAP = 8.dp
private val SPINNER_STROKE = 2.dp

/** “应用商店”：已审核的插件和社区插件分开列，可以按分类筛、按关键词找；上面是安装、升级、卸载的进展。 */
@Composable
fun StoreRoute(plugins: PluginsState, navigation: PluginNavigation, onBack: () -> Unit) {
    LaunchedEffect(plugins) { plugins.refresh(Action.RefreshStore(force = false)) }
    StoreScreen(plugins, navigation, onBack)
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun StoreScreen(plugins: PluginsState, navigation: PluginNavigation, onBack: () -> Unit) {
    var query by rememberSaveable { mutableStateOf("") }
    var tag by remember { mutableStateOf<String?>(null) }
    var showsUpdateAll by remember { mutableStateOf(false) }
    val viewModel = plugins.viewModel
    val store = viewModel.store
    val row: @Composable (StoreItem) -> Unit = { StoreRow(it, plugins, navigation) }
    WontonScreen(
        bar = {
            Column {
                WontonPushedBar(stringResource(R.string.store_title), onBack, showsStatusLine = true) {
                    if (viewModel.updatable.isNotEmpty()) {
                        UpdateAllButton(viewModel.updatable.size) { showsUpdateAll = true }
                    }
                    WontonIconButton(
                        WontonIcon.Add,
                        stringResource(R.string.store_add_source),
                        { navigation.addingSource = true },
                    )
                }
                WontonSearchField(
                    query,
                    { query = it },
                    stringResource(R.string.store_search_prompt),
                    Modifier.padding(
                        start = WontonTheme.spacing.pageInset,
                        end = WontonTheme.spacing.pageInset,
                        bottom = WontonTheme.spacing.s8,
                    ),
                )
            }
        },
    ) {
        Column(Modifier.fillMaxSize()) {
            PullToRefreshBox(
                // 第一次取的时候 StorePlaceholder 已经在转圈，这里不再转一个。
                isRefreshing = viewModel.refreshing && store != null,
                onRefresh = { plugins.send(Action.RefreshStore(force = true)) },
                modifier = Modifier.fillMaxSize(),
            ) {
                if (store == null) {
                    StorePlaceholder(plugins)
                } else {
                    LazyColumn(
                        Modifier.fillMaxSize(),
                        contentPadding = wontonContentPaddingValues(bottom = WontonTheme.spacing.s8),
                    ) {
                        items(viewModel.tasks, key = { "task:${it.id}" }) { task ->
                            Box(Modifier.padding(horizontal = WontonTheme.spacing.pageInset, vertical = ROW_GAP)) {
                                TaskRow(task, plugins) { navigation.confirmation = reconfirmation(task, plugins) }
                            }
                        }
                        viewModel.problem?.let { problem ->
                            item(key = "problem") {
                                Box(Modifier.padding(WontonTheme.spacing.pageInset)) {
                                    ProblemRow(problem.problem) {
                                        plugins.send(Action.RefreshStore(force = true))
                                    }
                                }
                            }
                        }
                        if (query.isEmpty()) {
                            listing(
                                store,
                                tag,
                                onTag = { tag = it },
                                onRetry = { plugins.send(Action.RefreshStore(force = true)) },
                            ) { row(it) }
                        } else {
                            results(plugins.core.searchStore(query)) { row(it) }
                        }
                    }
                }
            }
        }
    }

    // 全部更新只确认这一次（主人 2026-11-09 定）：列出可更新的应用，点“全部更新”逐个升级。
    if (showsUpdateAll && viewModel.updatable.isNotEmpty()) {
        UpdateAllSheet(plugins, viewModel.updatable, onDismiss = { showsUpdateAll = false }) {
            plugins.send(Action.UpgradeAll)
        }
    }
    // 核心判断要主人拿主意的一键操作：照着打开确认页，预览已经取好。
    LaunchedEffect(viewModel.confirmation) {
        navigation.confirmation = viewModel.confirmation?.let(PluginConfirmation::Store)
    }
}

/** 还没取到商店：取不到的原因和下一步，或者正在取。 */
@Composable
private fun StorePlaceholder(plugins: PluginsState) {
    val problem = plugins.viewModel.problem
    val rejection = plugins.refreshRejection
    val retry = { plugins.send(Action.RefreshStore(force = true)) }
    when {
        problem != null -> ProblemView(problem.problem, retry)
        rejection != null -> ProblemView(rejection.problem, retry)
        else -> ListSkeleton(WontonTheme.size.storeIcon)
    }
}

private fun LazyListScope.listing(
    store: StoreListing,
    tag: String?,
    onTag: (String?) -> Unit,
    onRetry: () -> Unit,
    row: @Composable (StoreItem) -> Unit,
) {
    val present = store.allPlugins.flatMap { it.tags }.distinct()
    if (present.size > 1) {
        item(key = "tags") { TagChips(present, tag, onTag) }
    }
    section(R.string.store_section_reviewed, store.official, tag, onRetry, row)
    added(store.added, tag, onRetry, row)
    section(R.string.store_section_community, store.community, tag, onRetry, row)
}

/** 主人添加的来源合成一段，取不到的来源写明是哪一个。 */
private fun LazyListScope.added(
    sources: List<AddedSource>,
    tag: String?,
    onRetry: () -> Unit,
    row: @Composable (StoreItem) -> Unit,
) {
    if (sources.isEmpty()) return
    item(key = "header:added") { StoreSectionTitle(stringResource(R.string.plugin_sources_mine)) }
    sources.forEach { source ->
        val shown = source.section.plugins.filter { tag == null || tag in it.tags }
        items(shown, key = { "added:${source.address}:${it.source}" }) { row(it) }
        unreadable(source.address, source.section.unreadable)
        val problem = source.section.problem
        if (problem != null || source.section.fetchedAtMs == null) {
            item(key = "problem:${source.address}") {
                Box(Modifier.padding(horizontal = WontonTheme.spacing.pageInset, vertical = ROW_GAP)) {
                    if (problem != null) {
                        val shownProblem = problem.problem
                        ProblemRow(
                            shownProblem.copy(
                                title = stringResource(
                                    R.string.store_problem_from_source,
                                    source.provider,
                                    shownProblem.titleText,
                                ),
                                titleRes = null,
                            ),
                            onRetry.takeIf { problem.canRetry },
                        )
                    } else {
                        // 核心重启后还没读过：不显示成空。
                        Text(
                            stringResource(R.string.store_reading, source.provider),
                            style = WontonTheme.typography.supporting,
                            color = WontonTheme.colors.textSecondary.toColor(),
                        )
                    }
                }
            }
        }
    }
}

private fun LazyListScope.section(
    titleRes: Int,
    content: StoreSection,
    tag: String?,
    onRetry: () -> Unit,
    row: @Composable (StoreItem) -> Unit,
) {
    val shown = content.plugins.filter { tag == null || tag in it.tags }
    if (shown.isEmpty() && content.problem == null && content.unreadable.isEmpty()) return
    item(key = "header:$titleRes") { StoreSectionTitle(stringResource(titleRes)) }
    items(shown, key = { "$titleRes:${it.source}" }) { row(it) }
    content.problem?.let { problem ->
        item(key = "problem:$titleRes") {
            Box(Modifier.padding(horizontal = WontonTheme.spacing.pageInset, vertical = ROW_GAP)) {
                ProblemRow(problem.problem, onRetry.takeIf { problem.canRetry })
            }
        }
    }
    unreadable("section:$titleRes", content.unreadable)
}

private fun LazyListScope.results(found: List<StoreItem>, row: @Composable (StoreItem) -> Unit) {
    if (found.isEmpty()) {
        item(key = "nothing") {
            Text(
                stringResource(R.string.store_no_results),
                Modifier.fillMaxWidth().padding(WontonTheme.spacing.s32),
                style = WontonTheme.typography.body.copy(textAlign = TextAlign.Center),
                color = WontonTheme.colors.textSecondary.toColor(),
            )
        }
    }
    items(found, key = { "found:${it.source}" }) { row(it) }
}

@Composable
private fun TagChips(
    tags: List<String>,
    selected: String?,
    onSelect: (String?) -> Unit,
) {
    LazyRow(
        contentPadding = PaddingValues(horizontal = WontonTheme.spacing.pageInset),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        item { WontonChip(stringResource(R.string.store_tag_all), selected == null) { onSelect(null) } }
        items(tags) { tag ->
            WontonChip(tag, selected == tag) { onSelect(tag) }
        }
    }
}

/** 商店里的一个插件：点开看详情，行尾的按钮安装、更新或打开。 */
@Composable
private fun StoreRow(item: StoreItem, plugins: PluginsState, navigation: PluginNavigation) {
    val colors = WontonTheme.colors
    Column {
        Row(
            Modifier.fillMaxWidth()
                .clickable { navigation.detail = PluginRoute.Store(item.source) }
                .padding(horizontal = WontonTheme.spacing.pageInset, vertical = ROW_GAP),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            PluginIconImage(item.icon, WontonTheme.size.storeIcon, plugins)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                Text(
                    item.title,
                    style = WontonTheme.typography.itemTitle,
                    color = colors.textPrimary.toColor(),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    item.summary,
                    style = WontonTheme.typography.supporting,
                    color = colors.textSecondary.toColor(),
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
                ItemTags(item)
            }
            when (val button = storeButton(item, plugins, navigation)) {
                is StoreButton.Get -> WontonSmallButton(button.title, button.perform, button.enabled)
                StoreButton.Busy ->
                    CircularProgressIndicator(Modifier.size(WontonTheme.size.icon), strokeWidth = SPINNER_STROKE)
                StoreButton.Hidden -> Unit
            }
        }
        // 一键操作在取预览时失败：行下面一行红字配“重试”，不弹窗（ADR-0017）。
        plugins.viewModel.tasks.firstOrNull { it.id == item.id }?.state
            ?.takeIf { !plugins.viewModel.hasRunningTask(item.id) && item.id !in plugins.viewModel.pending }
            ?.let { state ->
                if (state is PluginTaskState.Failed) {
                    val problem = state.reason.problem(state.message)
                    Row(
                        Modifier.padding(start = WontonTheme.size.storeIcon + WontonTheme.spacing.s12)
                            .padding(horizontal = WontonTheme.spacing.pageInset, vertical = ROW_GAP),
                        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Column(
                            Modifier.weight(1f),
                            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
                        ) {
                            Text(
                                problem.title,
                                style = WontonTheme.typography.supporting,
                                color = colors.danger.toColor(),
                            )
                            ProblemDetail(state.message)
                        }
                        WontonSmallButton(
                            stringResource(R.string.common_retry),
                            {
                                if (item.installedVersion == null) {
                                    plugins.send(Action.InstallNow(source = item.source))
                                } else {
                                    plugins.send(Action.UpgradeNow(pluginId = item.id))
                                }
                            },
                        )
                    }
                }
            }
    }
}

/** 已审核的插件在自己的分组里，不再标来源。 */
@Composable
private fun ItemTags(item: StoreItem) {
    val colors = WontonTheme.colors
    val showsTrust = item.trust != PluginTrust.OFFICIAL
    val stars = item.stars
    val provider = item.viaIndex?.let { stringResource(R.string.store_via_index, it) } ?: item.provider
    val hasSource = showsTrust || provider != null
    val highTier = item.securityTier.let { it != null && it != SecurityTier.MINIMAL && it != SecurityTier.STANDARD }
    if (hasSource || highTier || stars != null) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (showsTrust) Tag(item.trust.label, item.trust.tint)
            provider?.let {
                Text(
                    it,
                    style = WontonTheme.typography.caption,
                    color = colors.textSecondary.toColor(),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            if (highTier) Tag(stringResource(R.string.store_high_permissions), colors.danger)
            stars?.let {
                Row(
                    horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(
                        WontonIcon.StarFill,
                        contentDescription = stringResource(R.string.plugin_stars),
                        modifier = Modifier.size(WontonTheme.size.iconTiny),
                        tint = colors.textSecondary.toColor(),
                    )
                    Text(
                        CompactDecimalFormat.getInstance(
                            Locale.getDefault(),
                            CompactDecimalFormat.CompactStyle.SHORT,
                        ).format(it.toLong()),
                        style = WontonTheme.typography.caption,
                        color = colors.textSecondary.toColor(),
                    )
                }
            }
        }
    }
}

private sealed interface StoreButton {
    data class Get(val title: String, val enabled: Boolean, val perform: () -> Unit) : StoreButton

    data object Busy : StoreButton

    data object Hidden : StoreButton
}

/** 行尾的按钮：没装的“获取”，装着旧版本的“更新”，装着的“打开”；正在装、升级、卸载时转圈。 */
@Composable
private fun storeButton(item: StoreItem, plugins: PluginsState, navigation: PluginNavigation): StoreButton {
    val viewModel = plugins.viewModel
    val installed = item.installedVersion
    val entry = viewModel.installed?.plugins?.firstOrNull { it.id == item.id }?.entries?.firstOrNull()
    return when {
        // 取预览、装、升、卸都在转圈。
        viewModel.hasRunningTask(item.id) || item.id in viewModel.pending -> StoreButton.Busy
        installed == null -> StoreButton.Get(stringResource(R.string.store_get), item.blocked == null) {
            plugins.send(Action.InstallNow(source = item.source))
        }
        installed != item.version -> StoreButton.Get(stringResource(R.string.store_update), enabled = true) {
            plugins.send(Action.UpgradeNow(pluginId = item.id))
        }
        entry != null -> StoreButton.Get(stringResource(R.string.store_open), enabled = true) { navigation.open(entry) }
        else -> StoreButton.Hidden
    }
}
