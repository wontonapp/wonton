package app.wonton.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.res.stringResource
import kotlinx.coroutines.delay

@Composable
internal fun ConfirmationRemaining(expiresAtMs: Long): String {
    val now by produceState(System.currentTimeMillis(), expiresAtMs) {
        while (true) {
            value = System.currentTimeMillis()
            delay(1_000)
        }
    }
    val seconds = ((expiresAtMs - now).coerceAtLeast(0) + 999) / 1_000
    val duration = "%d:%02d".format(seconds / 60, seconds % 60)
    return stringResource(R.string.confirmations_remaining, duration)
}
