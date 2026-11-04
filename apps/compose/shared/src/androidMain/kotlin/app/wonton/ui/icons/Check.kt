package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Check: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Check",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 229.66 77.66 l -128.0 128.0 a 8.0 8.0 0.0 0 1 -11.32 0.0 l -56.0 -56.0 a 8.0 8.0 0.0 0 1 11.32 -11.32 L 96.0 188.69 L 218.34 66.34 a 8.0 8.0 0.0 0 1 11.32 11.32 Z")
        )
    }.build()
}
