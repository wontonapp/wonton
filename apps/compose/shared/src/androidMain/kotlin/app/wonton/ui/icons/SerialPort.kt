package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.SerialPort: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "SerialPort",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 149.66 138.34 a 8.0 8.0 0.0 0 0 -11.32 0.0 L 120.0 156.69 L 99.31 136.0 l 18.35 -18.34 a 8.0 8.0 0.0 0 0 -11.32 -11.32 L 88.0 124.69 L 69.66 106.34 a 8.0 8.0 0.0 0 0 -11.32 11.32 L 64.69 124.0 L 41.37 147.31 a 32.0 32.0 0.0 0 0 0.0 45.26 l 5.38 5.37 l -28.41 28.4 a 8.0 8.0 0.0 0 0 11.32 11.32 l 28.4 -28.41 l 5.37 5.38 a 32.0 32.0 0.0 0 0 45.26 0.0 L 132.0 191.31 l 6.34 6.35 a 8.0 8.0 0.0 0 0 11.32 -11.32 L 131.31 168.0 l 18.35 -18.34 A 8.0 8.0 0.0 0 0 149.66 138.34 Z M 97.37 203.34 a 16.0 16.0 0.0 0 1 -22.62 0.0 L 52.69 181.25 a 16.0 16.0 0.0 0 1 0.0 -22.62 L 76.0 135.31 L 120.69 180.0 Z M 237.66 18.34 a 8.0 8.0 0.0 0 0 -11.32 0.0 l -28.4 28.41 l -5.37 -5.38 a 32.05 32.05 0.0 0 0 -45.26 0.0 L 124.0 64.69 l -6.34 -6.35 a 8.0 8.0 0.0 0 0 -11.32 11.32 l 80.0 80.0 a 8.0 8.0 0.0 0 0 11.32 -11.32 L 191.31 132.0 l 23.32 -23.31 a 32.0 32.0 0.0 0 0 0.0 -45.26 l -5.38 -5.37 l 28.41 -28.4 A 8.0 8.0 0.0 0 0 237.66 18.34 Z M 203.31 97.34 L 180.0 120.69 L 135.31 76.0 l 23.32 -23.31 a 16.0 16.0 0.0 0 1 22.62 0.0 l 22.06 22.0 A 16.0 16.0 0.0 0 1 203.31 97.37 Z")
        )
    }.build()
}
