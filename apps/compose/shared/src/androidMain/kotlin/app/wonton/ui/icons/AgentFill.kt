package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.AgentFill: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "AgentFill",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 208.0 144.0 a 15.78 15.78 0.0 0 1 -10.42 14.94 L 146.0 178.0 l -19.0 51.62 a 15.92 15.92 0.0 0 1 -29.88 0.0 L 78.0 178.0 l -51.62 -19.0 a 15.92 15.92 0.0 0 1 0.0 -29.88 L 78.0 110.0 l 19.0 -51.62 a 15.92 15.92 0.0 0 1 29.88 0.0 L 146.0 110.0 l 51.62 19.0 A 15.78 15.78 0.0 0 1 208.0 144.0 Z M 152.0 48.0 h 16.0 L 168.0 64.0 a 8.0 8.0 0.0 0 0 16.0 0.0 L 184.0 48.0 h 16.0 a 8.0 8.0 0.0 0 0 0.0 -16.0 L 184.0 32.0 L 184.0 16.0 a 8.0 8.0 0.0 0 0 -16.0 0.0 L 168.0 32.0 L 152.0 32.0 a 8.0 8.0 0.0 0 0 0.0 16.0 Z M 240.0 80.0 h -8.0 L 232.0 72.0 a 8.0 8.0 0.0 0 0 -16.0 0.0 v 8.0 h -8.0 a 8.0 8.0 0.0 0 0 0.0 16.0 h 8.0 v 8.0 a 8.0 8.0 0.0 0 0 16.0 0.0 L 232.0 96.0 h 8.0 a 8.0 8.0 0.0 0 0 0.0 -16.0 Z")
        )
    }.build()
}
