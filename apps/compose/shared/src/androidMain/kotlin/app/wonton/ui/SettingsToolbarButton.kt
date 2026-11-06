package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Settings
import app.wonton.ui.icons.WontonIcon

@Composable
internal fun SettingsToolbarButton(upgrade: Boolean, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    Box {
        val label = if (upgrade) tabText("设置，有新版本", "Settings, update available") else tabText("设置", "Settings")
        WontonIconButton(WontonIcon.Settings, label, onClick)
        if (upgrade) {
            Box(
                Modifier.align(Alignment.TopEnd).padding(WontonTheme.spacing.s8).size(12.dp)
                    .background(colors.primary.toColor(), CircleShape)
                    .border(2.dp, colors.canvas.toColor(), CircleShape),
            )
        }
    }
}
