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
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.wonton.protocol.PendingConfirmation
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.Notify
import app.wonton.ui.icons.WontonIcon

/** 文字：只有一张卡片时说是什么事。 */
fun pendingCardsText(cards: List<PendingConfirmation>): String =
    if (cards.size == 1) "${cards[0].title}，等你确认" else "${cards.size} 张卡片等你确认"

/** 有卡片等主人回答时顶在页面最上面，点了去卡片所在的房间。多张时去最早到期的那张。 */
@Composable
fun PendingCardBanner(cards: List<PendingConfirmation>, onOpen: (String) -> Unit) {
    val colors = WontonTheme.colors
    val text = colors.textOnAccentFill.toColor()
    Column(
        Modifier
            .fillMaxWidth()
            .background(colors.accentFill.toColor())
            .clickable(role = Role.Button) { onOpen(cards[0].roomId) }
            .windowInsetsPadding(WindowInsets.statusBars),
    ) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = 48.dp).padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(WontonIcon.Notify, null, Modifier.size(20.dp), tint = text)
            Text(
                pendingCardsText(cards),
                Modifier.weight(1f),
                color = text,
                fontSize = 15.sp,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Icon(WontonIcon.Forward, null, Modifier.size(16.dp), tint = text)
        }
        colors.accentFillBorder?.let { border ->
            Box(Modifier.fillMaxWidth().height(1.dp).background(border.toColor()))
        }
    }
}
