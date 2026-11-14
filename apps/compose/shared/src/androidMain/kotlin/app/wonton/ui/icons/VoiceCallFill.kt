package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.VoiceCallFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "VoiceCallFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 231.88 175.08 A 56.26 56.26 0.0 0 1 176.0 224.0 C 96.6 224.0 32.0 159.4 32.0 80.0 A 56.26 56.26 0.0 0 1 80.92 24.12 a 16.0 16.0 0.0 0 1 16.62 9.52 l 21.12 47.15 l 0.0 0.12 A 16.0 16.0 0.0 0 1 117.39 96.0 c -0.18 0.27 -0.37 0.52 -0.57 0.77 L 96.0 121.45 c 7.49 15.22 23.41 31.0 38.83 38.51 l 24.34 -20.71 a 8.12 8.12 0.0 0 1 0.75 -0.56 a 16.0 16.0 0.0 0 1 15.17 -1.4 l 0.13 0.06 l 47.11 21.11 A 16.0 16.0 0.0 0 1 231.88 175.08 Z")
        )
    }.build()
}
