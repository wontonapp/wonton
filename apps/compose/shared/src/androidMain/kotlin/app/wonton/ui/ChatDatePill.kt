package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import java.text.DateFormat
import java.util.Date

/** 日期胶囊（ADR-0109 第 4 节）：列表里跨天处的那一枚和滚动时浮在顶上的那一枚长得一样。 */
@Composable
internal fun DatePill(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        modifier
            .background(
                WontonTheme.colors.surfaceSubtle.toColor().copy(alpha = PILL_ALPHA),
                RoundedCornerShape(WontonTheme.radii.pill),
            )
            .padding(horizontal = WontonTheme.spacing.s12, vertical = WontonTheme.spacing.s4),
        style = WontonTheme.typography.caption,
        color = WontonTheme.colors.textSecondary.toColor(),
    )
}

/** 胶囊的半透明底。 */
private const val PILL_ALPHA = 0.9f

/** 哪一天的日期，胶囊里写这个。 */
internal fun dateLabel(timestampMs: Long): String =
    DateFormat.getDateInstance(DateFormat.MEDIUM).format(Date(timestampMs))
