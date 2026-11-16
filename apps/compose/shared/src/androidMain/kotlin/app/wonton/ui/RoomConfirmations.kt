package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.ConfirmationView
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTheme

@Composable
internal fun RoomConfirmationRow(cards: List<ConfirmationView>, onOpen: (String) -> Unit) {
    val first = cards.firstOrNull() ?: return
    WontonRow(
        title = stringResource(R.string.confirmation_card_requests),
        badgeCount = cards.size,
        showsForward = true,
        onClick = { onOpen(first.card.id) },
    )
}

@Composable
internal fun RoomConfirmationSheet(
    cards: List<ConfirmationView>,
    openedId: String?,
    onDismiss: () -> Unit,
    onAction: (Action) -> Unit,
) {
    if (openedId == null) return
    WontonSheet(onDismiss, stringResource(R.string.confirmation_card_requests)) {
        key(openedId) {
            val listState = rememberLazyListState(
                initialFirstVisibleItemIndex = cards.indexOfFirst { it.card.id == openedId }.coerceAtLeast(0),
            )
            if (cards.none { it.card.id == openedId }) {
                Text(
                    stringResource(R.string.confirmation_card_unavailable),
                    Modifier.padding(WontonTheme.spacing.pageInset),
                    style = WontonTheme.typography.supporting,
                )
            }
            LazyColumn(
                state = listState,
                contentPadding = PaddingValues(horizontal = WontonTheme.spacing.pageInset),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
            ) {
                items(cards, key = { it.card.id }) { view ->
                    Box(Modifier.padding(bottom = WontonTheme.spacing.s8)) {
                        ConfirmationCardView(view, onAction)
                    }
                }
            }
        }
    }
}
