package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.CodeFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "CodeFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 216.0 40.0 L 40.0 40.0 A 16.0 16.0 0.0 0 0 24.0 56.0 L 24.0 200.0 a 16.0 16.0 0.0 0 0 16.0 16.0 L 216.0 216.0 a 16.0 16.0 0.0 0 0 16.0 -16.0 L 232.0 56.0 A 16.0 16.0 0.0 0 0 216.0 40.0 Z M 92.8 145.6 a 8.0 8.0 0.0 1 1 -9.6 12.8 l -32.0 -24.0 a 8.0 8.0 0.0 0 1 0.0 -12.8 l 32.0 -24.0 a 8.0 8.0 0.0 0 1 9.6 12.8 L 69.33 128.0 Z M 151.69 74.2 L 119.69 186.2 a 8.0 8.0 0.0 1 1 -15.38 -4.4 l 32.0 -112.0 a 8.0 8.0 0.0 0 1 15.38 4.4 Z M 204.8 134.4 L 172.8 158.4 a 8.0 8.0 0.0 0 1 -9.6 -12.8 L 186.67 128.0 L 163.2 110.4 a 8.0 8.0 0.0 1 1 9.6 -12.8 l 32.0 24.0 a 8.0 8.0 0.0 0 1 0.0 12.8 Z")
        )
    }.build()
}
