package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Back: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Back",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f,
        autoMirror = true
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 165.66 202.34 a 8.0 8.0 0.0 0 1 -11.32 11.32 l -80.0 -80.0 a 8.0 8.0 0.0 0 1 0.0 -11.32 l 80.0 -80.0 a 8.0 8.0 0.0 0 1 11.32 11.32 L 91.31 128.0 Z")
        )
    }.build()
}
