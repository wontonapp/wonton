package app.wonton.ui.design

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.WontonIcon

@Composable
fun StepOptionRow(
    title: String,
    selected: Boolean,
    onSelect: (() -> Unit)?,
    modifier: Modifier = Modifier,
    description: String? = null,
    blocked: String? = null,
    multiple: Boolean = false,
    leading: @Composable () -> Unit = {},
    trailing: @Composable () -> Unit = {},
) {
    val colors = WontonTheme.colors
    val shape = RoundedCornerShape(WontonTheme.radii.control)
    Row(
        modifier.fillMaxWidth().clip(shape)
            .then(
                if (selected) Modifier.accentFill(shape)
                else Modifier.background(colors.surfaceSubtle.toColor()),
            ),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Row(
            Modifier.weight(1f).heightIn(min = WontonTheme.size.target)
                .optionSelection(selected, onSelect, blocked, multiple)
                .padding(WontonTheme.spacing.s12),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            leading()
            Column(
                Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
            ) {
                val textColor = if (selected) colors.textOnAccentFill else colors.textPrimary
                Text(title, style = WontonTheme.typography.body, color = textColor.toColor())
                description?.let {
                    Text(
                        it,
                        style = WontonTheme.typography.supporting,
                        color = (if (selected) colors.textOnAccentFill else colors.textSecondary).toColor(),
                    )
                }
                blocked?.let {
                    Text(it, style = WontonTheme.typography.supporting, color = textColor.toColor())
                }
            }
            if (selected) {
                Icon(
                    WontonIcon.Check,
                    contentDescription = null,
                    modifier = Modifier.size(WontonTheme.size.iconSmall),
                    tint = colors.accentInkOnFill.toColor(),
                )
            }
        }
        // Keep independent actions outside the selection target and its disabled semantics.
        Box { trailing() }
    }
}

private fun Modifier.optionSelection(
    isSelected: Boolean,
    onSelect: (() -> Unit)?,
    blocked: String?,
    multiple: Boolean,
): Modifier = when {
    onSelect == null -> semantics(mergeDescendants = true) {
        selected = isSelected
        if (blocked != null) disabled()
    }
    multiple -> toggleable(
        value = isSelected,
        enabled = blocked == null,
        role = Role.Checkbox,
        onValueChange = { onSelect() },
    )
    else -> selectable(
        selected = isSelected,
        enabled = blocked == null,
        role = Role.RadioButton,
        onClick = onSelect,
    )
}
