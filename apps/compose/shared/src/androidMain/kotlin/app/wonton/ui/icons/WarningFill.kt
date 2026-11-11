package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.WarningFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "WarningFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 236.8 188.09 L 149.35 36.22 h 0.0 a 24.76 24.76 0.0 0 0 -42.7 0.0 L 19.2 188.09 a 23.51 23.51 0.0 0 0 0.0 23.72 A 24.35 24.35 0.0 0 0 40.55 224.0 h 174.9 a 24.35 24.35 0.0 0 0 21.33 -12.19 A 23.51 23.51 0.0 0 0 236.8 188.09 Z M 120.0 104.0 a 8.0 8.0 0.0 0 1 16.0 0.0 v 40.0 a 8.0 8.0 0.0 0 1 -16.0 0.0 Z M 128.0 192.0 a 12.0 12.0 0.0 1 1 12.0 -12.0 A 12.0 12.0 0.0 0 1 128.0 192.0 Z")
        )
    }.build()
}
