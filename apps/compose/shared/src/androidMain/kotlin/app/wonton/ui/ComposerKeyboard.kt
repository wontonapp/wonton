package app.wonton.ui

import android.view.KeyEvent as NativeKeyEvent
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.nativeKeyEvent
import androidx.compose.ui.input.key.onPreviewKeyEvent

/** IME-generated Return and active composition stay with the editor, not the send shortcut. */
@Composable
internal fun Modifier.hardwareMessageKeys(state: TextFieldState, onSend: (String) -> Unit): Modifier {
    var handledReturn by remember(state) { mutableStateOf(false) }
    return onPreviewKeyEvent { event ->
        val native = event.nativeKeyEvent
        val isReturn = native.keyCode == NativeKeyEvent.KEYCODE_ENTER ||
            native.keyCode == NativeKeyEvent.KEYCODE_NUMPAD_ENTER
        if (!isReturn || native.flags and NativeKeyEvent.FLAG_SOFT_KEYBOARD != 0 ||
            native.device?.isVirtual != false
        ) {
            return@onPreviewKeyEvent false
        }
        if (native.action == NativeKeyEvent.ACTION_UP) {
            val consumed = handledReturn
            handledReturn = false
            return@onPreviewKeyEvent consumed
        }
        if (native.action != NativeKeyEvent.ACTION_DOWN || state.composition != null ||
            native.isCtrlPressed || native.isAltPressed || native.isMetaPressed
        ) return@onPreviewKeyEvent false
        handledReturn = true
        if (native.repeatCount == 0) {
            if (native.isShiftPressed) {
                state.edit {
                    val start = selection.min
                    replace(start, selection.max, "\n")
                    placeCursorAfterCharAt(start)
                }
            } else {
                state.text.toString().takeIf { it.isNotBlank() }?.let(onSend)
            }
        }
        true
    }
}
