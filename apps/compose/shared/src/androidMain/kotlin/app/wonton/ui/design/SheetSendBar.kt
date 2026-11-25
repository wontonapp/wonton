package app.wonton.ui.design

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import app.wonton.ui.R
import app.wonton.ui.icons.Send
import app.wonton.ui.icons.WontonIcon

/**
 * 面板底部的发送条（照 Telegram）：右边主题色圆形发送键，键上是选中的个数，键盘弹起时跟着上来。
 * `note` 不为 null 时左边还有附言框（附件、转发面板）。没有选中东西时键是灰的，`busy` 时也灰着。
 */
@Composable
fun SheetSendBar(
    note: String?,
    onNoteChange: (String) -> Unit,
    count: Int,
    hint: String,
    onSend: () -> Unit,
    busy: Boolean = false,
    sendTag: String = "sheet-send",
) {
    val colors = WontonTheme.colors
    val sendLabel = stringResource(R.string.composer_send)
    val focusRequester = remember { FocusRequester() }
    LaunchedEffect(count) { if (count > 0 && note != null) focusRequester.requestFocus() }
    Row(
        Modifier.fillMaxWidth().testTag(sendTag).imePadding()
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s8),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (note != null && count > 0) {
            Box(
                Modifier.weight(1f)
                    .heightIn(min = WontonTheme.size.target)
                    .background(colors.surfaceSubtle.toColor(), RoundedCornerShape(WontonTheme.radii.control))
                    .padding(horizontal = WontonTheme.spacing.s12),
                contentAlignment = Alignment.CenterStart,
            ) {
                BasicTextField(
                    value = note,
                    onValueChange = onNoteChange,
                    singleLine = true,
                    textStyle = WontonTheme.typography.body.copy(color = colors.textPrimary.toColor()),
                    cursorBrush = SolidColor(colors.textPrimary.toColor()),
                    modifier = Modifier.fillMaxWidth().focusRequester(focusRequester),
                    decorationBox = { field ->
                        if (note.isEmpty()) {
                            Text(hint, style = WontonTheme.typography.body, color = colors.textSecondary.toColor())
                        }
                        field()
                    },
                )
            }
        } else {
            Box(Modifier.weight(1f))
        }
        val enabled = count > 0 && !busy
        Box(
            Modifier.size(WontonTheme.size.target)
                .testTag(sendTag)
                .clickable(role = Role.Button, enabled = enabled, onClick = onSend)
                .semantics { contentDescription = sendLabel },
            contentAlignment = Alignment.Center,
        ) {
            Box(
                Modifier.size(WontonTheme.size.sendButton)
                    .background((if (enabled) colors.primary else colors.surfaceSubtle).toColor(), CircleShape),
                contentAlignment = Alignment.Center,
            ) {
                if (enabled) {
                    Text(count.toString(), style = WontonTheme.typography.itemTitle, color = colors.onPrimary.toColor())
                } else {
                    Icon(
                        WontonIcon.Send,
                        // 装饰：外层的可点区域已经带了“发送”。
                        contentDescription = null,
                        modifier = Modifier.size(WontonTheme.size.iconSmall),
                        tint = colors.textSecondary.toColor(),
                    )
                }
            }
        }
    }
}
