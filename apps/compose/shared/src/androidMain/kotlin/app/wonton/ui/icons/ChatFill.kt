package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.ChatFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "ChatFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 232.07 186.76 a 80.0 80.0 0.0 0 0 -62.5 -114.17 A 80.0 80.0 0.0 1 0 23.93 138.76 l -7.27 24.71 a 16.0 16.0 0.0 0 0 19.87 19.87 l 24.71 -7.27 a 80.39 80.39 0.0 0 0 25.18 7.35 a 80.0 80.0 0.0 0 0 108.34 40.65 l 24.71 7.27 a 16.0 16.0 0.0 0 0 19.87 -19.86 Z M 215.82 188.23 L 224.0 216.0 l -27.76 -8.17 a 8.0 8.0 0.0 0 0 -6.0 0.63 a 64.05 64.05 0.0 0 1 -85.87 -24.88 A 79.93 79.93 0.0 0 0 174.7 89.71 a 64.0 64.0 0.0 0 1 41.75 92.48 A 8.0 8.0 0.0 0 0 215.82 188.23 Z")
        )
    }.build()
}
