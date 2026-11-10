package app.wonton.ui.design

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

/**
 * 自己画的底部面板（ADR-0026）：手势和屏幕阅读器用系统的，样子自己定。往下拖、点空白处、按返回键走 [onDismiss]。
 * 内容拿到的 `close` 先把面板收起来，收完才执行传入的操作，操作要弹别的页面时不会撞上。
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WontonSheet(
    onDismiss: () -> Unit,
    title: String? = null,
    content: @Composable ColumnScope.(close: (() -> Unit) -> Unit) -> Unit,
) {
    val colors = WontonTheme.colors
    val spacing = WontonTheme.spacing
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    val close: (() -> Unit) -> Unit = { action ->
        scope.launch { state.hide() }.invokeOnCompletion {
            onDismiss()
            action()
        }
    }
    HideKeyboardOnAppear()
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = state,
        shape = RoundedCornerShape(topStart = WontonTheme.radii.card, topEnd = WontonTheme.radii.card),
        containerColor = colors.surface.toColor(),
        contentColor = colors.textPrimary.toColor(),
        tonalElevation = 0.dp,
        scrimColor = Color.Black.copy(alpha = WontonTheme.ratio.scrim),
        dragHandle = {
            Box(
                Modifier.padding(vertical = spacing.s8)
                    .size(WontonTheme.size.grabberWidth, WontonTheme.size.grabberHeight)
                    .background(colors.separator.toColor(), CircleShape),
            )
        },
    ) {
        WontonFontScale {
            title?.let {
                Text(
                    it,
                    Modifier.padding(start = spacing.pageInset, end = spacing.pageInset, bottom = spacing.s8)
                        .semantics { heading() },
                    style = WontonTheme.typography.itemTitle,
                    color = colors.textPrimary.toColor(),
                )
            }
            content(close)
            Box(Modifier.padding(bottom = spacing.s8))
        }
    }
}

/** 列一组操作的底部面板。 */
@Composable
fun WontonActionSheet(actions: List<WontonSheetAction>, onDismiss: () -> Unit, title: String? = null) {
    val colors = WontonTheme.colors
    WontonSheet(onDismiss, title) { close ->
        actions.forEach { action ->
            val tint = (if (action.isDanger) colors.danger else colors.textPrimary).toColor()
            Row(
                Modifier.fillMaxWidth()
                    .heightIn(min = WontonTheme.size.target)
                    .clickable(role = Role.Button) { close(action.onClick) }
                    .padding(horizontal = WontonTheme.spacing.pageInset),
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    action.icon,
                    // 装饰：操作项有文字。
                    contentDescription = null,
                    modifier = Modifier.size(WontonTheme.size.icon),
                    tint = tint,
                )
                Text(action.title, style = WontonTheme.typography.body, color = tint)
            }
        }
    }
}
