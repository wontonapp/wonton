package app.wonton.ui.design

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.DialogWindowProvider

/** 键盘开着时弹窗口，窗口的高度算不对；关掉之后焦点回到输入框，键盘又会自己弹出来。所以先收起键盘。 */
@Composable
internal fun HideKeyboardOnAppear() {
    val focus = LocalFocusManager.current
    LaunchedEffect(Unit) { focus.clearFocus() }
}

/** 自己画的弹窗（ADR-0026）。点空白处和返回键走 [onDismiss]，不给 [actions] 时只有一个“好”，也走它。 */
@Composable
fun WontonDialog(
    title: String,
    message: String? = null,
    onDismiss: () -> Unit,
    actions: List<WontonDialogAction> = listOf(WontonDialogAction("好", onClick = onDismiss)),
    detail: (@Composable () -> Unit)? = null,
) {
    val colors = WontonTheme.colors
    val spacing = WontonTheme.spacing
    HideKeyboardOnAppear()
    Dialog(onDismiss, DialogProperties(usePlatformDefaultWidth = false)) {
        (LocalView.current.parent as? DialogWindowProvider)?.window?.setDimAmount(WontonTheme.ratio.scrim)
        WontonFontScale {
            Column(
                Modifier.padding(spacing.pageInset)
                    .widthIn(max = WontonTheme.size.dialogWidth)
                    .fillMaxWidth()
                    .background(colors.surface.toColor(), RoundedCornerShape(WontonTheme.radii.card))
                    .padding(spacing.s16),
                verticalArrangement = Arrangement.spacedBy(spacing.s16),
            ) {
                Column(
                    Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(spacing.s8),
                ) {
                    Text(
                        title,
                        Modifier.semantics { heading() },
                        style = WontonTheme.typography.itemTitle,
                        color = colors.textPrimary.toColor(),
                    )
                    message?.let {
                        Text(it, style = WontonTheme.typography.body, color = colors.textSecondary.toColor())
                    }
                    detail?.invoke()
                }
                if (actions.size == 2) {
                    Row(horizontalArrangement = Arrangement.spacedBy(spacing.s8)) {
                        actions.forEach { WontonButton(it.title, it.onClick, Modifier.weight(1f), it.kind) }
                    }
                } else {
                    Column(verticalArrangement = Arrangement.spacedBy(spacing.s8)) {
                        actions.forEach { WontonButton(it.title, it.onClick, Modifier.fillMaxWidth(), it.kind) }
                    }
                }
            }
        }
    }
}
