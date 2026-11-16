package app.wonton.ui

import androidx.compose.runtime.Composable
import app.wonton.protocol.PageAction
import app.wonton.protocol.Action
import app.wonton.protocol.TabPage
import app.wonton.protocol.TabRef
import app.wonton.protocol.TabState

@Composable
internal fun BotPageDisclosure(
    page: TabPage, active: Boolean, awaitingLogin: Boolean, pages: PagesState,
    onContinue: () -> Unit, cancel: (() -> Unit)? = null,
) {
    val ready = page.state is TabState.Ready && page.url != null
    if (active && awaitingLogin && ready) {
        BotWebLoginDisclosure(
            onContinue = onContinue,
            onCancel = {
            cancel?.invoke() ?: pages.send(Action.Pages(PageAction.ClosePage))
            },
        )
    }
}
