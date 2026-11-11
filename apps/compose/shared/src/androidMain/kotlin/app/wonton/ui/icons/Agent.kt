package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Agent: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Agent",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 197.58 129.06 L 146.0 110.0 l -19.0 -51.62 a 15.92 15.92 0.0 0 0 -29.88 0.0 L 78.0 110.0 l -51.62 19.0 a 15.92 15.92 0.0 0 0 0.0 29.88 L 78.0 178.0 l 19.0 51.62 a 15.92 15.92 0.0 0 0 29.88 0.0 L 146.0 178.0 l 51.62 -19.0 a 15.92 15.92 0.0 0 0 0.0 -29.88 Z M 137.0 164.22 a 8.0 8.0 0.0 0 0 -4.74 4.74 L 112.0 223.85 L 91.78 169.0 A 8.0 8.0 0.0 0 0 87.0 164.22 L 32.15 144.0 L 87.0 123.78 A 8.0 8.0 0.0 0 0 91.78 119.0 L 112.0 64.15 L 132.22 119.0 a 8.0 8.0 0.0 0 0 4.74 4.74 L 191.85 144.0 Z M 144.0 40.0 a 8.0 8.0 0.0 0 1 8.0 -8.0 h 16.0 V 16.0 a 8.0 8.0 0.0 0 1 16.0 0.0 V 32.0 h 16.0 a 8.0 8.0 0.0 0 1 0.0 16.0 H 184.0 V 64.0 a 8.0 8.0 0.0 0 1 -16.0 0.0 V 48.0 H 152.0 A 8.0 8.0 0.0 0 1 144.0 40.0 Z M 248.0 88.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 h -8.0 v 8.0 a 8.0 8.0 0.0 0 1 -16.0 0.0 V 96.0 h -8.0 a 8.0 8.0 0.0 0 1 0.0 -16.0 h 8.0 V 72.0 a 8.0 8.0 0.0 0 1 16.0 0.0 v 8.0 h 8.0 A 8.0 8.0 0.0 0 1 248.0 88.0 Z")
        )
    }.build()
}
