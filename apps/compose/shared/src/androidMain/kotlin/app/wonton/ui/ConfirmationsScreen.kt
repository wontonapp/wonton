package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.wontonContentPadding
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
                rejection = core.send(Action.RetryConfirmations)
            }
            else -> Column(
                Modifier.fillMaxSize().verticalScroll(rememberScrollState()).wontonContentPadding()
                    .padding(WontonTheme.spacing.pageInset),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
            ) {
                view.problem?.let { problem -> ProblemView(problem.problem) {
                    rejection = core.send(Action.RetryConfirmations)
                } }
                if (cards.isEmpty()) Text(stringResource(R.string.confirmations_empty))
                cards.forEach { card -> ConfirmationCardView(card) { rejection = core.send(it) } }
            }
        }
    }
    rejection?.let { ProblemDialog(it) { rejection = null } }
}
