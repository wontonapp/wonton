package app.wonton.ui.design

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Rgb
import app.wonton.ui.R
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.WontonIcon

/** 一组列表行的底。 */
@Composable
fun WontonGroup(content: @Composable () -> Unit) {
    Column(
        Modifier.fillMaxWidth()
            .clip(RoundedCornerShape(WontonTheme.radii.card))
            .background(WontonTheme.colors.surfaceSubtle.toColor()),
    ) {
        content()
    }
}

/** 列表行，有 `onClick` 时能点。 */
@Composable
fun WontonRow(
    title: String,
    value: String? = null,
    valueTint: Rgb? = null,
    showsForward: Boolean = false,
    titleTint: Rgb? = null,
    badge: Boolean = false,
    onClick: (() -> Unit)? = null,
) {
    val colors = WontonTheme.colors
    val badgeState = stringResource(R.string.design_update_badge)
    Row(
        Modifier.fillMaxWidth()
            .heightIn(min = WontonTheme.size.target)
            .then(if (onClick != null) Modifier.clickable(role = Role.Button, onClick = onClick) else Modifier)
            .then(if (badge) Modifier.semantics { stateDescription = badgeState } else Modifier)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Text(title, style = WontonTheme.typography.body, color = (titleTint ?: colors.textPrimary).toColor())
        Box(Modifier.weight(1f), contentAlignment = Alignment.CenterEnd) {
            value?.let {
                Text(
                    it,
                    style = WontonTheme.typography.body.copy(textAlign = TextAlign.End),
                    color = (valueTint ?: colors.textSecondary).toColor(),
                )
            }
        }
        if (badge) {
            Box(
                Modifier.size(12.dp)
                    .background(colors.primary.toColor(), CircleShape)
                    .border(2.dp, colors.canvas.toColor(), CircleShape),
            )
        }
        if (showsForward) {
            Icon(
                WontonIcon.Forward,
                // 装饰：箭头只表示这一行能点。
                contentDescription = null,
                modifier = Modifier.size(WontonTheme.size.iconSmall),
                tint = colors.textSecondary.toColor(),
            )
        }
    }
}

@Composable
fun WontonSeparator() {
    Box(
        Modifier.padding(start = WontonTheme.spacing.s16)
            .fillMaxWidth()
            .height(WontonTheme.size.hairline)
            .background(WontonTheme.colors.separator.toColor()),
    )
}
