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
        if (!native.isPhysicalReturn()) return@onPreviewKeyEvent false
        if (native.action == NativeKeyEvent.ACTION_UP) {
            val consumed = handledReturn
            handledReturn = false
            return@onPreviewKeyEvent consumed
        }
        if (!native.isSendKeyDown(state)) return@onPreviewKeyEvent false
        handledReturn = true
        if (native.repeatCount == 0) sendOrInsertNewline(native, state, onSend)
        true
    }
}

private fun NativeKeyEvent.isPhysicalReturn(): Boolean {
    val returnKey = keyCode == NativeKeyEvent.KEYCODE_ENTER || keyCode == NativeKeyEvent.KEYCODE_NUMPAD_ENTER
    val physicalDevice = device?.isVirtual == false
    val notSoft = flags and NativeKeyEvent.FLAG_SOFT_KEYBOARD == 0
    return returnKey && physicalDevice && notSoft
}

private fun NativeKeyEvent.isSendKeyDown(state: TextFieldState): Boolean {
    val plainModifiers = !isCtrlPressed && !isAltPressed && !isMetaPressed
    val noComposition = state.composition == null
    return action == NativeKeyEvent.ACTION_DOWN && plainModifiers && noComposition
}

private fun sendOrInsertNewline(event: NativeKeyEvent, state: TextFieldState, onSend: (String) -> Unit) {
    if (event.isShiftPressed) {
        state.edit {
            val start = selection.min
            replace(start, selection.max, "\n")
            placeCursorAfterCharAt(start)
        }
    } else {
        state.text.toString().takeIf { it.isNotBlank() }?.let(onSend)
    }
}
