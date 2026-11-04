package app.wonton.ui.design

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp

/** 主界面的底栏（ADR-0026）：浮起的表面加噪点，选中的一项是重点色的填充和实心图标。 */
@Composable
fun WontonBottomBar(items: List<WontonBarItem>, selected: Int, onSelect: (Int) -> Unit) {
    val colors = WontonTheme.colors
    val shape = RoundedCornerShape(WontonTheme.radii.bar)
    Row(
        Modifier.navigationBarsPadding()
            .padding(start = WontonTheme.spacing.s16, end = WontonTheme.spacing.s16, bottom = WontonTheme.spacing.s12)
            .fillMaxWidth()
            .shadow(3.dp, shape)
            .background(colors.surface.toColor(), shape)
            .noise(shape)
            .border(WontonTheme.size.hairline, colors.separator.toColor(), shape)
            .padding(WontonTheme.spacing.s8),
    ) {
        items.forEachIndexed { index, item ->
            val isSelected = index == selected
            val content = (if (isSelected) colors.accentInkOnFill else colors.textSecondary).toColor()
            val itemShape = RoundedCornerShape(WontonTheme.radii.barItem)
            Column(
                Modifier.weight(1f)
                    .heightIn(min = WontonTheme.size.target)
                    .clip(itemShape)
                    .then(if (isSelected) Modifier.accentFill(itemShape, border = false) else Modifier)
                    .selectable(selected = isSelected, role = Role.Tab, onClick = { onSelect(index) })
                    .padding(vertical = WontonTheme.spacing.s4),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(2.dp, Alignment.CenterVertically),
            ) {
                Icon(
                    if (isSelected) item.selectedIcon else item.icon,
                    contentDescription = null,
                    modifier = Modifier.size(WontonTheme.size.icon),
                    tint = content,
                )
                Text(item.label, style = WontonTheme.typography.caption, color = content)
            }
        }
    }
}
