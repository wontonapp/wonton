package app.wonton.ui.design

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Search
import app.wonton.ui.icons.WontonIcon

private const val PRESSED_ALPHA = 0.6f
private const val DISABLED_ALPHA = 0.4f

/** 搜索框：放大镜在前，有内容时能清空。 */
@Composable
fun WontonSearchField(value: String, onValueChange: (String) -> Unit, prompt: String, modifier: Modifier = Modifier) {
    val colors = WontonTheme.colors
    BasicTextField(
        value = value,
        onValueChange = onValueChange,
        singleLine = true,
        textStyle = WontonTheme.typography.body.copy(color = colors.textPrimary.toColor()),
        cursorBrush = SolidColor(colors.textPrimary.toColor()),
        modifier = modifier.fillMaxWidth(),
        decorationBox = { field ->
            Row(
                Modifier.heightIn(min = WontonTheme.size.target)
                    .background(colors.surfaceSubtle.toColor(), RoundedCornerShape(WontonTheme.radii.control))
                    .padding(start = WontonTheme.spacing.s12),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                Icon(
                    WontonIcon.Search,
                    contentDescription = null,
                    modifier = Modifier.size(WontonTheme.size.iconSmall),
                    tint = colors.textSecondary.toColor(),
                )
                Box(Modifier.weight(1f)) {
                    if (value.isEmpty()) {
                        Text(prompt, style = WontonTheme.typography.body, color = colors.textSecondary.toColor())
                    }
                    field()
                }
                if (value.isNotEmpty()) {
                    WontonIconButton(WontonIcon.Close, "清除", { onValueChange("") }, colors.textSecondary.toColor())
                }
            }
        },
    )
}

/** 筛选用的小胶囊：选中的用重点色。图形比按钮矮，命中区还是 48。 */
@Composable
fun WontonChip(title: String, isSelected: Boolean, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    Box(
        Modifier.heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button, onClick = onClick, indication = null, interactionSource = null)
            .semantics { selected = isSelected },
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier.heightIn(min = WontonTheme.size.chip)
                .then(
                    if (isSelected) {
                        Modifier.accentFill(CircleShape)
                    } else {
                        Modifier.background(colors.surfaceSubtle.toColor(), CircleShape)
                    },
                )
                .padding(horizontal = WontonTheme.spacing.s12),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                title,
                style = WontonTheme.typography.supporting,
                color = (if (isSelected) colors.textOnAccentFill else colors.textPrimary).toColor(),
            )
        }
    }
}

/** 列表行尾的小按钮。图形比按钮矮，命中区还是 48。 */
@Composable
fun WontonSmallButton(text: String, onClick: () -> Unit, enabled: Boolean = true) {
    val colors = WontonTheme.colors
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    Box(
        Modifier.heightIn(min = WontonTheme.size.target)
            .alpha(if (!enabled) DISABLED_ALPHA else if (pressed) PRESSED_ALPHA else 1f)
            .clickable(interaction, indication = null, enabled = enabled, role = Role.Button, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier.heightIn(min = WontonTheme.size.smallButton)
                .background(colors.surfaceSubtle.toColor(), CircleShape)
                .padding(horizontal = WontonTheme.spacing.s12),
            contentAlignment = Alignment.Center,
        ) {
            Text(text, style = WontonTheme.typography.supporting, color = colors.textPrimary.toColor())
        }
    }
}

/** 开关：语义还是 `toggleable`，样子自己画。 */
@Composable
fun WontonSwitch(
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    enabled: Boolean = true,
    modifier: Modifier = Modifier,
) {
    val colors = WontonTheme.colors
    val inset = (WontonTheme.size.switchHeight - WontonTheme.size.switchKnob) / 2
    val travel = WontonTheme.size.switchWidth - WontonTheme.size.switchKnob - inset
    val offset by animateDpAsState(
        if (checked) travel else inset,
        tween(WontonTheme.motion.feedback),
        label = "switch",
    )
    Box(
        modifier.heightIn(min = WontonTheme.size.target)
            .alpha(if (enabled) 1f else DISABLED_ALPHA)
            .toggleable(checked, enabled = enabled, role = Role.Switch, onValueChange = onCheckedChange),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier.size(WontonTheme.size.switchWidth, WontonTheme.size.switchHeight)
                .background((if (checked) colors.primary else colors.separator).toColor(), CircleShape),
            contentAlignment = Alignment.CenterStart,
        ) {
            Box(
                Modifier.offset(x = offset)
                    .size(WontonTheme.size.switchKnob)
                    .background((if (checked) colors.onPrimary else colors.textSecondary).toColor(), CircleShape),
            )
        }
    }
}
