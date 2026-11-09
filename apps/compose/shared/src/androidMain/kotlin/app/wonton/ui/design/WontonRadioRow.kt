package app.wonton.ui.design

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextAlign
import app.wonton.protocol.Rgb
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.WontonIcon

private const val DISABLED_ALPHA = 0.4f
private const val RESULT_WEIGHT = 0.65f

/** Radio row with independently wrapping title and result, including long hostnames. */
@Composable
fun WontonRadioRow(
    title: String,
    selected: Boolean,
    enabled: Boolean,
    onSelect: () -> Unit,
    value: String? = null,
    valueTint: Rgb? = null,
) {
    val colors = WontonTheme.colors
    val shape = RoundedCornerShape(WontonTheme.radii.control)
    Row(
        Modifier.fillMaxWidth().clip(shape)
            .then(if (selected) Modifier.accentFill(shape) else Modifier.background(colors.surfaceSubtle.toColor()))
            .alpha(if (enabled) 1f else DISABLED_ALPHA)
            .heightIn(min = WontonTheme.size.target)
            .selectable(selected, enabled = enabled, role = Role.RadioButton, onClick = onSelect)
            .padding(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Text(
            title, Modifier.weight(1f), style = WontonTheme.typography.body,
            color = (if (selected) colors.textOnAccentFill else colors.textPrimary).toColor(),
        )
        value?.let {
            Text(
                it, Modifier.weight(RESULT_WEIGHT),
                style = WontonTheme.typography.supporting.copy(textAlign = TextAlign.End),
                color = (valueTint ?: if (selected) colors.textOnAccentFill else colors.textSecondary).toColor(),
            )
        }
        if (selected) {
            Icon(
                WontonIcon.Check, contentDescription = null, modifier = Modifier.size(WontonTheme.size.iconSmall),
                tint = colors.accentInkOnFill.toColor(),
            )
        }
    }
}
