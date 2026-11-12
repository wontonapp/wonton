package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.BotView
import app.wonton.client.ClientCore
import app.wonton.protocol.ConfirmationsViewModel
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.icons.CheckCircle
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

fun ClientCore.confirmationsUpdates(): Flow<ConfirmationsViewModel> = flow {
    subscribeConfirmations().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}

/** “等你确认”列表（ADR-0016）：代替原来 Wonton 私聊里的卡片记录。等回答的在前、结束 24 小时内的在后，
 * 每张能点开回答。取不到列表时上次的还在，照实提示并给重试。 */
@Composable
fun ConfirmationsScreen(
    view: ConfirmationsViewModel,
    bots: List<BotView>,
    plugins: PluginsState,
    onAction: (Action) -> Unit,
    onClose: () -> Unit,
) {
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.confirmations_title), onClose) }) {
        val problem = view.problem
        when {
            problem != null && view.pending.isEmpty() && view.finished.isEmpty() ->
                ProblemView(problem.problem) { onAction(Action.RetryConfirmations) }
            view.loading ->
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
            view.pending.isEmpty() && view.finished.isEmpty() -> EmptyConfirmations()
            else -> ConfirmationsList(view, bots, plugins, onAction)
        }
    }
}

@Composable
private fun ConfirmationsList(
    view: ConfirmationsViewModel,
    bots: List<BotView>,
    plugins: PluginsState,
    onAction: (Action) -> Unit,
) {
    val cards = view.pending + view.finished
    LazyColumn(Modifier.fillMaxSize().wontonContentPadding()) {
        view.problem?.let { problem ->
            item(key = "problem") {
                ProblemRow(problem.problem) { onAction(Action.RetryConfirmations) }
                WontonSeparator()
            }
        }
        itemsIndexed(cards, key = { _, view -> view.card.id }) { index, card ->
            if (index > 0 || view.problem != null) {
                Box(Modifier.padding(start = WontonTheme.size.avatar + WontonTheme.spacing.s12)) {
                    WontonSeparator()
                }
            }
            ConfirmationCardRow(card, bots, plugins, onAction)
        }
    }
}

@Composable
private fun EmptyConfirmations() {
    val colors = WontonTheme.colors
    Column(
        Modifier.fillMaxSize().padding(WontonTheme.spacing.s32),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8, Alignment.CenterVertically),
    ) {
        Icon(
            WontonIcon.CheckCircle,
            contentDescription = null,
            modifier = Modifier.size(WontonTheme.size.icon),
            tint = colors.textSecondary.toColor(),
        )
        Text(
            stringResource(R.string.confirmations_empty),
            style = WontonTheme.typography.supporting,
            color = colors.textSecondary.toColor(),
        )
    }
}
