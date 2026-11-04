package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.PluginCategory
import app.wonton.protocol.PluginTaskState
import app.wonton.protocol.PluginTrust
import app.wonton.protocol.StoreItem
import app.wonton.protocol.StoreListing
import app.wonton.protocol.StoreSection

/** “商店”：已审核的插件和社区插件分开列，可以按分类筛、按关键词找；上面是安装、升级、卸载的进展。 */
@Composable
fun StoreRoute(plugins: PluginsState, navigation: PluginNavigation) {
    LaunchedEffect(plugins) { plugins.refresh(Action.RefreshStore(force = false)) }
    PluginPages(plugins, navigation) { StoreScreen(plugins, navigation) }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun StoreScreen(plugins: PluginsState, navigation: PluginNavigation) {
    var query by rememberSaveable { mutableStateOf("") }
    var category by remember { mutableStateOf<PluginCategory?>(null) }
    val viewModel = plugins.viewModel
    val row: @Composable (StoreItem) -> Unit = { StoreRow(it, plugins, navigation) }
    Scaffold(topBar = { TopAppBar(title = { Text("商店") }) }) { padding ->
        Column(Modifier.padding(padding).fillMaxSize()) {
            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp),
                placeholder = { Text("搜索") },
                singleLine = true,
                shape = CircleShape,
            )
            PullToRefreshBox(
                isRefreshing = viewModel.refreshing,
                onRefresh = { plugins.send(Action.RefreshStore(force = true)) },
                modifier = Modifier.fillMaxSize(),
            ) {
                val store = viewModel.store
                if (store == null) {
                    StorePlaceholder(plugins)
                } else {
                    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(vertical = 8.dp)) {
                        items(viewModel.tasks, key = { "task:${it.id}" }) { task ->
                            TaskRow(task, plugins) { navigation.confirmation = reconfirmation(task, plugins) }
                        }
                        viewModel.problem?.let { problem ->
                            item(key = "problem") {
                                Box(Modifier.padding(16.dp)) { ProblemRow(problem.message, problem.reason.nextStep) }
                            }
                        }
                        if (query.isEmpty()) {
                            listing(store, category, onCategory = { category = it }) { row(it) }
                        } else {
                            results(plugins.core.searchStore(query)) { row(it) }
                        }
                    }
                }
            }
        }
    }
}

/** 还没取到商店：取不到的原因和下一步，或者正在取。 */
@Composable
private fun StorePlaceholder(plugins: PluginsState) {
    val problem = plugins.viewModel.problem
    val rejection = plugins.refreshRejection
    val retry = { plugins.send(Action.RefreshStore(force = true)) }
    when {
        problem != null -> ProblemView(problem.message, problem.reason.nextStep, retry)
        rejection != null -> ProblemView(rejection, null, retry)
        else -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
    }
}

private fun LazyListScope.listing(
    store: StoreListing,
    category: PluginCategory?,
    onCategory: (PluginCategory?) -> Unit,
    row: @Composable (StoreItem) -> Unit,
) {
    val present = (store.official.plugins + store.community.plugins).mapNotNull { it.category }.distinct()
    if (present.size > 1) {
        item(key = "categories") { CategoryChips(present, category, onCategory) }
    }
    section("已审核", store.official, category, row)
    section("社区", store.community, category, row)
}

private fun LazyListScope.section(
    title: String,
    content: StoreSection,
    category: PluginCategory?,
    row: @Composable (StoreItem) -> Unit,
) {
    val shown = content.plugins.filter { category == null || it.category == category }
    if (shown.isEmpty() && content.problem == null) return
    item(key = "header:$title") {
        Text(
            title,
            Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
            style = MaterialTheme.typography.titleLarge,
        )
    }
    items(shown, key = { "$title:${it.source}" }) { row(it) }
    content.problem?.let { problem ->
        item(key = "problem:$title") {
            Text(
                problem.message,
                Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

private fun LazyListScope.results(found: List<StoreItem>, row: @Composable (StoreItem) -> Unit) {
    if (found.isEmpty()) {
        item(key = "nothing") {
            Text("没有找到", Modifier.fillMaxWidth().padding(32.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
    items(found, key = { "found:${it.source}" }) { row(it) }
}

@Composable
private fun CategoryChips(
    categories: List<PluginCategory>,
    selected: PluginCategory?,
    onSelect: (PluginCategory?) -> Unit,
) {
    LazyRow(
        contentPadding = PaddingValues(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item { FilterChip(selected = selected == null, onClick = { onSelect(null) }, label = { Text("全部") }) }
        items(categories) { category ->
            FilterChip(
                selected = selected == category,
                onClick = { onSelect(category) },
                label = { Text(category.label) },
            )
        }
    }
}

/** 商店里的一个插件：点开看详情，行尾的按钮安装、更新或打开。 */
@Composable
private fun StoreRow(item: StoreItem, plugins: PluginsState, navigation: PluginNavigation) {
    Row(
        Modifier.fillMaxWidth()
            .clickable { navigation.detail = PluginRoute.Store(item.source) }
            .padding(horizontal = 16.dp, vertical = 10.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        PluginIconImage(item.icon, item.category, 56.dp, plugins)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                item.title,
                style = MaterialTheme.typography.titleMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                item.summary,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            ItemTags(item)
        }
        when (val button = storeButton(item, plugins, navigation)) {
            is StoreButton.Get -> FilledTonalButton(onClick = button.perform, enabled = button.enabled) {
                Text(button.title)
            }
            StoreButton.Busy -> CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
            StoreButton.Hidden -> Unit
        }
    }
}

/** 已审核的插件在自己的分组里，不再标来源。 */
@Composable
private fun ItemTags(item: StoreItem) {
    val showsTrust = item.trust != PluginTrust.OFFICIAL
    val stars = item.stars
    if (showsTrust || item.highPermissions || stars != null) {
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
            if (showsTrust) Tag(item.trust.label, item.trust.tint)
            if (item.highPermissions) Tag("高权限", Red)
            stars?.let { Text("★ $it", style = MaterialTheme.typography.labelSmall, color = Gray) }
        }
    }
}

private sealed interface StoreButton {
    data class Get(val title: String, val enabled: Boolean, val perform: () -> Unit) : StoreButton

    data object Busy : StoreButton

    data object Hidden : StoreButton
}

/** 行尾的按钮：没装的“获取”，装着旧版本的“更新”，装着的“打开”；正在装、升级、卸载时转圈。 */
private fun storeButton(item: StoreItem, plugins: PluginsState, navigation: PluginNavigation): StoreButton {
    val viewModel = plugins.viewModel
    val installed = item.installedVersion
    val entry = viewModel.installed?.plugins?.firstOrNull { it.id == item.id }?.entries?.firstOrNull()
    return when {
        viewModel.tasks.any { it.id == item.id && it.state is PluginTaskState.Running } -> StoreButton.Busy
        installed == null -> StoreButton.Get("获取", item.blocked == null) {
            navigation.confirmation = PluginConfirmation.Install(item.source)
        }
        installed != item.version -> StoreButton.Get("更新", enabled = true) {
            navigation.confirmation = PluginConfirmation.Upgrade(item.id)
        }
        entry != null -> StoreButton.Get("打开", enabled = true) { navigation.open(entry) }
        else -> StoreButton.Hidden
    }
}
