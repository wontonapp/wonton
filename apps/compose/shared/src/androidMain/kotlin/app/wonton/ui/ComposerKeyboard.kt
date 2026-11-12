package app.wonton.ui

import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isAltPressed
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.isShiftPressed

import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
/** IME-generated Return and active composition stay with the editor, not the send shortcut. */
@Composable
internal fun Modifier.hardwareMessageKeys(state: TextFieldState, onSend: (String) -> Unit): Modifier {
    var handledReturn by remember(state) { mutableStateOf(false) }
    return onPreviewKeyEvent { event ->
        if (event.key != Key.Enter) return@onPreviewKeyEvent false
        if (event.type == KeyEventType.KeyUp) {
            val consumed = handledReturn
            handledReturn = false
            return@onPreviewKeyEvent consumed
        }
        if (event.type != KeyEventType.KeyDown || state.composition != null) {
            return@onPreviewKeyEvent false
        }
        if (event.isCtrlPressed || event.isAltPressed || event.isMetaPressed) {
            return@onPreviewKeyEvent false
        }
        if (handledReturn) return@onPreviewKeyEvent true
        handledReturn = true
        if (event.isShiftPressed) {
            state.edit {
                val start = selection.min
                replace(start, selection.max, "\n")
                placeCursorAfterCharAt(start)
            }
        } else {
            state.text.toString().takeIf { it.isNotBlank() }?.let(onSend)
        }
        true
    }
}
