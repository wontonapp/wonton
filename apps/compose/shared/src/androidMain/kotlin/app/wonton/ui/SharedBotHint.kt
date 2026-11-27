package app.wonton.ui

import android.content.Context
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.ui.design.toColor
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.WontonIcon

/** 共享 Bot 的长期记忆全家共用：第一次私聊时提示一次，关了就按房间记住。 */
@Composable
fun SharedBotMemoryHint(onDismiss: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(
            start = WontonTheme.spacing.pageInset,
            end = WontonTheme.spacing.pageInset,
            top = WontonTheme.spacing.s8,
            bottom = WontonTheme.spacing.s8,
        ),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        verticalAlignment = Alignment.Top,
    ) {
        Icon(
            WontonIcon.Info,
            contentDescription = null,
            modifier = Modifier.size(WontonTheme.size.iconSmall),
            tint = WontonTheme.colors.textSecondary.toColor(),
        )
        Text(
            stringResource(R.string.family_sharedBotMemory),
            Modifier.weight(1f),
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        Icon(
            WontonIcon.Close,
            contentDescription = stringResource(R.string.family_dismiss),
            modifier = Modifier
                .size(WontonTheme.size.target)
                .padding(WontonTheme.spacing.s8)
                .clickable(onClick = onDismiss),
            tint = WontonTheme.colors.textSecondary.toColor(),
        )
    }
}

object SharedBotHintPrefs {
    private const val FILE = "shared_bot_hints"

    fun seen(context: Context, roomId: String, botUserId: String): Boolean =
        prefs(context).getBoolean("$roomId|$botUserId", false)

    fun dismiss(context: Context, roomId: String, botUserId: String) {
        prefs(context).edit().putBoolean("$roomId|$botUserId", true).apply()
    }

    private fun prefs(context: Context) = context.getSharedPreferences(FILE, Context.MODE_PRIVATE)
}
