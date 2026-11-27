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
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.TextAutoSize
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
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.sp
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.WontonIcon

/** 窄按钮里字最小的字号：再小就换省略号，不缩到看不清。 */
private val labelledOptionMinFontSize = 13.sp

@Composable
fun StepOptionRow(
    title: String,
    selected: Boolean,
    onSelect: (() -> Unit)?,
    modifier: Modifier = Modifier,
    description: String? = null,
    blocked: String? = null,
    neutralSelection: Boolean = false,
    multiple: Boolean = false,
    leading: @Composable () -> Unit = {},
    trailing: @Composable () -> Unit = {},
    centered: Boolean = false,
) {
    val colors = WontonTheme.colors
    val shape = RoundedCornerShape(WontonTheme.radii.control)
    // 中性选中不铺强调色：选中只由勾选标志表达，和装不了的红字分开。
    val emphasisFilled = selected && !neutralSelection
    Row(
        modifier.fillMaxWidth().clip(shape)
            .then(
                if (emphasisFilled) Modifier.emphasisFill(shape)
                else Modifier.background(colors.surfaceSubtle.toColor()),
            ),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Row(
            Modifier.weight(1f).heightIn(min = WontonTheme.size.target)
                .optionSelection(selected, onSelect, blocked, multiple)
                // 挤在一行的窄按钮把内边距收窄，先拿边距换字的地方。
                .padding(
                    horizontal = if (centered) WontonTheme.spacing.s4 else WontonTheme.spacing.s12,
                    vertical = WontonTheme.spacing.s12,
                ),
            // 挤在一行的窄按钮：表情、字和勾之间留半个字宽，别贴在一起。
            horizontalArrangement = if (centered) {
                Arrangement.spacedBy(WontonTheme.spacing.s8, Alignment.CenterHorizontally)
            } else {
                Arrangement.spacedBy(WontonTheme.spacing.s12)
            },
            verticalAlignment = Alignment.CenterVertically,
        ) {
            leading()
            Column(
                if (centered) Modifier else Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
            ) {
                val textColor = if (emphasisFilled) colors.onPrimary else colors.textPrimary
                // 挤在一行的窄按钮（消息下的选项）：表情和字一起居中，放不下就省略号，不折行。
                if (centered) {
                    // 挤在一行的窄按钮：放不下先缩字，13sp 是底，再放不下才省略号。
                    BasicText(
                        title,
                        style = WontonTheme.typography.body.copy(
                            color = textColor.toColor(),
                            textAlign = TextAlign.Center,
                        ),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        autoSize = TextAutoSize.StepBased(
                            minFontSize = labelledOptionMinFontSize,
                            maxFontSize = WontonTheme.typography.body.fontSize,
                        ),
                    )
                } else {
                    Text(
                        title,
                        style = WontonTheme.typography.body,
                        color = textColor.toColor(),
                        overflow = TextOverflow.Clip,
                        textAlign = TextAlign.Start,
                    )
                }
                description?.let {
                    Text(
                        it,
                        style = WontonTheme.typography.supporting,
                        color = (if (emphasisFilled) colors.onPrimary else colors.textSecondary).toColor(),
                    )
                }
                blocked?.let {
                    Text(it, style = WontonTheme.typography.supporting, color = colors.danger.toColor())
                }
            }
            if (selected) {
                Icon(
                    WontonIcon.Check,
                    // 装饰：选中状态由选项本身朗读。
                    contentDescription = null,
                    modifier = Modifier.size(WontonTheme.size.iconSmall),
                    tint = (if (neutralSelection) colors.primary else colors.onPrimary).toColor(),
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
    // 没有动作就不是能点的选项：读屏念“停用”，原来的选中状态照念；`blocked` 只是失败原因，写在正文里。
    onSelect == null -> semantics(mergeDescendants = true) {
        selected = isSelected
        disabled()
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
