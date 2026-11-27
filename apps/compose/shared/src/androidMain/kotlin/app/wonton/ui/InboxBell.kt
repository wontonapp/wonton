package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import app.wonton.protocol.InboxViewModel
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Inbox
import app.wonton.ui.icons.WontonIcon

/** 铃铛上的数：等你回答的条数。还没取到、或最近一次没取成时不确定，是 null，不当成 0。 */
internal val InboxViewModel.needing: Int?
    get() = if (loading || problem != null) null else list.counts.needing.toInt()

/**
 * 主界面根页顶栏的收件箱铃铛（ADR-0119 第 9 节）：有等你回答的才在角上画数字；没有就是一枚普通的铃铛，不画 0。
 * 数字不确定时也不画，进去看页面上的问题。
 */
@Composable
internal fun InboxBell(needing: Int?, onOpen: () -> Unit) {
    val count = needing?.takeIf { it > 0 }
    val state = count?.let { stringResource(R.string.design_badge_count, it) }
    Box(
        Modifier.testTag("inbox-bell")
            .then(if (state != null) Modifier.semantics { stateDescription = state } else Modifier),
    ) {
        WontonIconButton(WontonIcon.Inbox, stringResource(R.string.inbox_title), onOpen)
        if (count != null) {
            Box(
                Modifier.align(Alignment.TopEnd).offset(x = (-4).dp, y = 4.dp)
                    .background(WontonTheme.colors.danger.toColor(), CircleShape)
                    .padding(horizontal = 5.dp),
            ) {
                Text(
                    count.toString(),
                    style = WontonTheme.typography.caption,
                    color = androidx.compose.ui.graphics.Color.White,
                )
            }
        }
    }
}
