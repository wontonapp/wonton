package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Forward: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Forward",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f,
        autoMirror = true
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 181.66 133.66 l -80.0 80.0 a 8.0 8.0 0.0 0 1 -11.32 -11.32 L 164.69 128.0 L 90.34 53.66 a 8.0 8.0 0.0 0 1 11.32 -11.32 l 80.0 80.0 A 8.0 8.0 0.0 0 1 181.66 133.66 Z")
        )
    }.build()
}
