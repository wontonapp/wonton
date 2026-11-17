package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.BotContactAction
import app.wonton.protocol.ChatAction
import app.wonton.protocol.BotView
import app.wonton.protocol.ConfirmationsViewModel
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.icons.CheckCircle
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.flow.flow

/** 没有房间的确认通知也要能处理，复用会话里的卡片和核心的同一份列表。 */
@Composable
internal fun ConfirmationsScreen(core: ClientCore, cardId: String, onBack: () -> Unit) {
    val updates = remember(core) {
        flow {
            core.subscribeConfirmations().use { subscription ->
                while (true) emit(subscription.next() ?: break)
            }
        }
    }
    val view by updates.collectAsState(remember(core) { core.confirmations() })
    var rejection by remember { mutableStateOf<Problem?>(null) }
    BackHandler(onBack = onBack)
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.confirmations_title), onBack) }) {
        val cards = (view.pending + view.finished).sortedBy { it.card.id != cardId }
        when {
            view.loading -> LoadingView()
            view.problem != null && cards.isEmpty() -> ProblemView(view.problem!!.problem) {
                rejection = core.send(Action.Chat(ChatAction.RetryConfirmations))
            }
            else -> Column(
                Modifier.fillMaxSize().verticalScroll(rememberScrollState()).wontonContentPadding()
                    .padding(WontonTheme.spacing.pageInset),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
            ) {
                view.problem?.let { problem -> ProblemView(problem.problem) {
                    rejection = core.send(Action.Chat(ChatAction.RetryConfirmations))
                } }
                if (cards.isEmpty()) Text(stringResource(R.string.confirmations_empty))
                cards.forEach { card -> ConfirmationCardView(card) { rejection = core.send(it) } }
            }
        }
    }
    rejection?.let { ProblemDialog(it) { rejection = null } }
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
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.confirmations_title), onClose) }) {
        val problem = view.problem
        when {
            problem != null && view.pending.isEmpty() && view.finished.isEmpty() && view.contactRequests.isEmpty() ->
                ProblemView(problem.problem) { onAction(Action.Chat(ChatAction.RetryConfirmations)) }
            view.loading ->
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
            view.pending.isEmpty() && view.finished.isEmpty() && view.contactRequests.isEmpty() &&
                contactView.actionFailure == null &&
                contactView.state !is app.wonton.protocol.BotContactsState.Failed -> EmptyConfirmations()
            else -> ConfirmationsList(view, bots, plugins, send, contacts, contactView)
        }
    }
}

@Composable
private fun ConfirmationsList(
    view: ConfirmationsViewModel,
    bots: List<BotView>,
    plugins: PluginsState,
    onAction: (Action) -> Unit,
    contacts: ConfirmationContactsState,
    contactView: app.wonton.protocol.BotContactsViewModel,
) {
    val cards = view.pending + view.finished
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
                ProblemRow(problem.problem) { onAction(Action.Chat(ChatAction.RetryConfirmations)) }
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
