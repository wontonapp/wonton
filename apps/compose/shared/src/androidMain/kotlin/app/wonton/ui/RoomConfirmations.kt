package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.protocol.Action
import app.wonton.protocol.ConfirmationView
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.Notify
import app.wonton.ui.icons.WontonIcon

/** 等回答的卡片在输入框上方：底色强调色，一行写“有事要你定”，下一行是第一张的标题，点开面板（ADR-0036）。 */
@Composable
internal fun RoomConfirmationRow(cards: List<ConfirmationView>, onOpen: (String) -> Unit) {
    val first = cards.firstOrNull() ?: return
    val ink = WontonTheme.colors.textOnAccentFill.toColor()
    val icon = Modifier.size(WontonTheme.size.iconSmall)
    Row(
        Modifier
            .fillMaxWidth()
            .background(WontonTheme.colors.accentFill.toColor())
            .clickable(role = Role.Button) { onOpen(first.card.id) }
            .heightIn(min = WontonTheme.size.target)
            .padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s8)
            .testTag("room-confirmations"),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        Icon(WontonIcon.Notify, null, icon, tint = ink)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(
                stringResource(R.string.confirmation_card_requests),
                style = WontonTheme.typography.caption,
                color = ink,
            )
            Text(
                first.card.title,
                style = WontonTheme.typography.supporting,
                color = ink,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Icon(WontonIcon.Forward, null, icon, tint = ink)
    }
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
