package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.PluginAction
import app.wonton.protocol.StoreUpdateBatch
import app.wonton.protocol.StoreUpdateItem
import app.wonton.protocol.StoreUpdateState
import app.wonton.protocol.UpdatablePlugin
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonSearchField
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.Download
import app.wonton.ui.icons.WontonIcon

/** 商店右上角的更新图标（向下的箭头），有可更新时才出现，带红色数字角标。 */
@Composable
internal fun UpdateAllButton(count: Int, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    val label = stringResource(R.string.store_update_all_a11y, count)
    Box(
        Modifier.size(WontonTheme.size.target)
            .clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            WontonIcon.Download,
            // 装饰：按钮外层已经带了名字。
            contentDescription = null,
            Modifier.size(WontonTheme.size.icon),
            tint = colors.textPrimary.toColor(),
        )
        Text(
            if (count > UPDATE_ALL_BADGE_MAX) "$UPDATE_ALL_BADGE_MAX+" else count.toString(),
            Modifier.align(Alignment.TopEnd)
                .offset(x = UPDATE_BADGE_OFFSET_X, y = UPDATE_BADGE_OFFSET_Y)
                .background(colors.danger.toColor(), CircleShape)
                .padding(horizontal = 5.dp),
            style = WontonTheme.typography.caption.copy(fontWeight = FontWeight.Bold),
            color = Color.White,
        )
    }
}

/** 全部更新前唯一的一次确认：列出要更新的应用和新版本号，防误触。 */
@Composable
internal fun UpdateAllSheet(
    plugins: PluginsState,
    updating: List<UpdatablePlugin>,
    onDismiss: () -> Unit,
    onConfirm: () -> Unit,
) {
    val colors = WontonTheme.colors
    val spacing = WontonTheme.spacing
    val title = pluralStringResource(R.plurals.store_update_all_title, updating.size, updating.size)
    WontonSheet(onDismiss, title) { close ->
        Column(
            Modifier.fillMaxWidth().padding(horizontal = spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(spacing.s16),
        ) {
            Column(verticalArrangement = Arrangement.spacedBy(spacing.s12)) {
                updating.forEach { app ->
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(spacing.s12),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        PluginIconImage(app.icon, WontonTheme.size.taskIcon, plugins)
                        Text(
                            app.title,
                            Modifier.weight(1f),
                            style = WontonTheme.typography.body,
                            color = colors.textPrimary.toColor(),
                        )
                        Text(
                            app.version,
                            style = WontonTheme.typography.supporting,
                            color = colors.textSecondary.toColor(),
                        )
                    }
                }
            }
            WontonButton(
                stringResource(R.string.store_update_all),
                { close { onConfirm() } },
                Modifier.fillMaxWidth(),
            )
        }
    }
}

/** Batch feedback stays above the independently scrolling catalog. */
@Composable
internal fun StoreUpdateBatchSection(
    batch: StoreUpdateBatch,
    plugins: PluginsState,
    navigation: PluginNavigation,
    modifier: Modifier = Modifier,
) {
    val colors = WontonTheme.colors
    val spacing = WontonTheme.spacing
    Column(
        modifier.fillMaxWidth().padding(horizontal = spacing.pageInset, vertical = spacing.s8),
        verticalArrangement = Arrangement.spacedBy(spacing.s8),
    ) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(spacing.s8),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                if (batch.finished) {
                    stringResource(
                        R.string.store_update_batch_finished,
                        batch.succeeded.toInt(),
                        batch.failed.toInt(),
                        batch.needsAttention.toInt(),
                    )
                } else {
                    stringResource(
                        R.string.store_update_batch_running,
                        (batch.succeeded + batch.failed).toInt(),
                        batch.items.size,
                    )
                },
                Modifier.weight(1f),
                style = WontonTheme.typography.supporting,
                color = colors.textPrimary.toColor(),
            )
            if (batch.finished) {
                WontonSmallButton(stringResource(R.string.store_update_batch_clear), {
                    plugins.send(Action.Plugins(PluginAction.DismissStoreUpdates))
                })
            }
        }
        LazyColumn(
            Modifier.fillMaxWidth().weight(1f, fill = false),
            verticalArrangement = Arrangement.spacedBy(spacing.s12),
        ) {
            items(batch.items, key = { it.app.pluginId }) { item ->
                StoreUpdateBatchRow(item, plugins, navigation)
            }
        }
    }
}

@Composable
private fun StoreUpdateBatchRow(
    item: StoreUpdateItem,
    plugins: PluginsState,
    navigation: PluginNavigation,
) {
    val colors = WontonTheme.colors
    val spacing = WontonTheme.spacing
    when (val state = item.state) {
        is StoreUpdateState.Task -> Column(
            verticalArrangement = Arrangement.spacedBy(spacing.s4),
        ) {
            Text(
                stringResource(R.string.store_update_batch_target, item.app.version),
                style = WontonTheme.typography.caption,
                color = colors.textSecondary.toColor(),
            )
            TaskRow(
                state.task,
                plugins,
                { navigation.confirmation = reconfirmation(state.task, plugins) },
                navigation.checkInstalled,
                showsDismiss = false,
            )
        }
        else -> Row(
            horizontalArrangement = Arrangement.spacedBy(spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            PluginIconImage(item.app.icon, WontonTheme.size.taskIcon, plugins)
            Column(
                Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(spacing.s4),
            ) {
                Text(
                    item.app.title,
                    style = WontonTheme.typography.itemTitle,
                    color = colors.textPrimary.toColor(),
                )
                Text(
                    stringResource(R.string.store_update_batch_target, item.app.version),
                    style = WontonTheme.typography.caption,
                    color = colors.textSecondary.toColor(),
                )
                Text(
                    stringResource(when (state) {
                        is StoreUpdateState.Queued -> R.string.store_update_batch_queued
                        is StoreUpdateState.Checking -> R.string.store_update_batch_checking
                        else -> R.string.store_update_batch_needs_decision
                    }),
                    style = WontonTheme.typography.supporting,
                    color = colors.textSecondary.toColor(),
                )
                if (state is StoreUpdateState.NeedsDecision) {
                    val reviewLabel = stringResource(R.string.store_update_batch_review_app, item.app.title)
                    Box(modifier = Modifier.semantics { contentDescription = reviewLabel }) {
                        WontonSmallButton(
                            stringResource(R.string.store_update_batch_review),
                            { plugins.send(Action.Plugins(PluginAction.ReviewStoreUpdate(item.app.pluginId))) },
                        )
                    }
                }
            }
            if (state is StoreUpdateState.Checking) {
                CircularProgressIndicator(Modifier.size(WontonTheme.size.icon), strokeWidth = 2.dp)
            }
        }
    }
}

private const val UPDATE_ALL_BADGE_MAX = 99

private val UPDATE_BADGE_OFFSET_X = (-4).dp
private val UPDATE_BADGE_OFFSET_Y = 6.dp
@Composable
internal fun StoreToolbar(
    plugins: PluginsState,
    navigation: PluginNavigation,
    query: String,
    onQuery: (String) -> Unit,
    onUpdate: () -> Unit,
    onBack: () -> Unit,
) {
    val viewModel = plugins.viewModel
    Column {
        WontonPushedBar(stringResource(R.string.store_title), onBack, showsStatusLine = true) {
            if (viewModel.updatable.isNotEmpty() && viewModel.updateBatch?.finished != false) {
                UpdateAllButton(viewModel.updatable.size, onUpdate)
            }
            WontonIconButton(
                WontonIcon.Add,
                stringResource(R.string.store_add_source),
                { navigation.addingSource = true },
            )
        }
        WontonSearchField(
            query,
            onQuery,
            stringResource(R.string.store_search_prompt),
            Modifier.padding(
                start = WontonTheme.spacing.pageInset,
                end = WontonTheme.spacing.pageInset,
                bottom = WontonTheme.spacing.s8,
            ),
        )
    }
}
