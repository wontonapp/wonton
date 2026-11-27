package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.BotContactAction
import app.wonton.protocol.BotView
import app.wonton.protocol.ChatAction
import app.wonton.protocol.InboxListItem
import app.wonton.protocol.InboxStatus
import app.wonton.protocol.InboxViewModel
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.icons.CheckCircle
import app.wonton.ui.icons.WontonIcon

/**
 * 收件箱（ADR-0119）：要你决定和要你知道的事。确认卡片是其中一类，点开能直接回答；联系人请求排在最前。
 * 顺序是核心排好的（等着的在前，已处理的在后）。取不到时上次的还在，照实提示并给重试。
 */
@Composable
fun InboxScreen(
    view: InboxViewModel,
    bots: List<BotView>,
    plugins: PluginsState,
    onAction: (Action) -> Unit,
    onClose: () -> Unit,
) {
    val contacts = remember { ConfirmationContactsState() }
    val contactView by remember(plugins.core) { plugins.core.botContactsUpdates() }
        .collectAsState(plugins.core.botContacts())
    val send: (Action) -> Unit = { action ->
        val rejection = plugins.core.send(action)
        if (rejection != null) {
            contacts.pending = null
            plugins.reject(rejection)
        }
    }
    ConfirmationContactEffects(plugins, contacts)
    val empty = view.entries.isEmpty() && view.contactRequests.isEmpty()
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.inbox_title), onClose) }) {
        val problem = view.problem
        when {
            problem != null && empty -> ProblemView(problem.problem) { onAction(Action.Chat(ChatAction.RetryInbox)) }
            view.loading && empty ->
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
            empty && contactView.actionFailure == null &&
                contactView.state !is app.wonton.protocol.BotContactsState.Failed -> EmptyInbox()
            else -> InboxList(view, bots, plugins, send, contacts, contactView)
        }
    }
}

@Composable
private fun InboxList(
    view: InboxViewModel,
    bots: List<BotView>,
    plugins: PluginsState,
    onAction: (Action) -> Unit,
    contacts: ConfirmationContactsState,
    contactView: app.wonton.protocol.BotContactsViewModel,
) {
    LazyColumn(Modifier.fillMaxSize().wontonContentPadding()) {
        contactView.actionFailure?.let { failure ->
            item(key = "contact-failure") { ProblemRow(failure.problem()) }
        }
        (contactView.state as? app.wonton.protocol.BotContactsState.Failed)?.let { failure ->
            item(key = "contact-load-failure") {
                ProblemRow(failure.problem()) {
                    contactView.bot?.let { onAction(Action.BotContacts(BotContactAction.RefreshBotContacts(it))) }
                }
            }
        }
        items(view.contactRequests.size, key = {
            val request = view.contactRequests[it]
            "contact:${request.bot.pluginId}:${request.bot.bot}:${request.contact.userId}"
        }) { index -> ConfirmationContactRow(view.contactRequests[index], bots, contacts, onAction) }
        view.problem?.let { problem ->
            item(key = "problem") {
                ProblemRow(problem.problem) { onAction(Action.Chat(ChatAction.RetryInbox)) }
                WontonSeparator()
            }
        }
        if (view.loading) item(key = "loading") { CircularProgressIndicator(Modifier.padding(WontonTheme.spacing.s16)) }
        itemsIndexed(view.entries, key = { _, entry -> entry.item.id }) { index, entry ->
            if (index > 0 || view.problem != null) {
                Box(Modifier.padding(start = WontonTheme.size.avatar + WontonTheme.spacing.s12)) {
                    WontonSeparator()
                }
            }
            val card = entry.card
            if (card != null) ConfirmationCardRow(card, bots, plugins, onAction) else InboxSummaryRow(entry.item)
        }
    }
}

/** 收件箱里只有摘要的一条：标题、发送方和纯文本正文（核心给旧 App、通知和读屏的兜底）。 */
@Composable
private fun InboxSummaryRow(item: InboxListItem) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth().testTag("inbox-item-${item.id}")
            .semantics(mergeDescendants = true) {}
            .padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s12),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(
                item.title,
                style = WontonTheme.typography.itemTitle,
                color = (if (item.status == InboxStatus.PENDING) colors.textPrimary else colors.textSecondary)
                    .toColor(),
            )
            Text(
                item.sender.displayName,
                style = WontonTheme.typography.supporting,
                color = colors.textSecondary.toColor(),
            )
            if (item.body.isNotEmpty()) {
                Text(
                    item.body,
                    style = WontonTheme.typography.supporting,
                    color = colors.textSecondary.toColor(),
                    maxLines = 3,
                )
            }
        }
        if (item.unread) {
            Box(
                Modifier.padding(top = WontonTheme.spacing.s8).size(8.dp)
                    .background(colors.primary.toColor(), CircleShape),
            )
        }
    }
}

@Composable
private fun EmptyInbox() {
    val colors = WontonTheme.colors
    Column(
        Modifier.fillMaxSize().padding(WontonTheme.spacing.s32).testTag("inbox-empty"),
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
            stringResource(R.string.inbox_empty),
            style = WontonTheme.typography.supporting,
            color = colors.textSecondary.toColor(),
        )
    }
}
