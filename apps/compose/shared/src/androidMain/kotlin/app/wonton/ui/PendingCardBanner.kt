package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.protocol.PendingConfirmation
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.Notify
import app.wonton.ui.icons.WontonIcon

/** 文字：只有一张卡片时说是什么事。 */
@Composable
internal fun pendingCardsText(cards: List<PendingConfirmation>): String =
    if (cards.size == 1) {
        stringResource(R.string.onboarding_pending_card_one, cards[0].title)
    } else {
        pluralStringResource(R.plurals.onboarding_pending_card_many, cards.size, cards.size)
    }

/** 有卡片等主人回答时顶在页面最上面，点了去卡片所在的房间。多张时去最早到期的那张。 */
@Composable
fun PendingCardBanner(cards: List<PendingConfirmation>, onOpen: (String) -> Unit) {
    val colors = WontonTheme.colors
    val text = colors.textOnAccentFill.toColor()
    Column(
        Modifier
            .fillMaxWidth()
            .background(colors.accentFill.toColor())
            // 没有会话的卡片该去“等你确认”列表；列表页还没有，先不跳。
            .clickable(role = Role.Button) { cards[0].roomId?.let(onOpen) }
            .windowInsetsPadding(WindowInsets.statusBars),
    ) {
        Row(
            Modifier.fillMaxWidth()
                .heightIn(min = WontonTheme.size.target)
                .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        ) {
            // 装饰：提示文字在右边。
            Icon(WontonIcon.Notify, null, Modifier.size(WontonTheme.size.iconSmall), tint = text)
            Text(
                pendingCardsText(cards),
                Modifier.weight(1f),
                style = WontonTheme.typography.supporting,
                color = text,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            // 装饰：整条可点，箭头只表示能过去。
            Icon(WontonIcon.Forward, null, Modifier.size(WontonTheme.size.iconSmall), tint = text)
        }
        colors.accentFillBorder?.let { border ->
            Box(Modifier.fillMaxWidth().height(WontonTheme.size.hairline).background(border.toColor()))
        }
    }
}
