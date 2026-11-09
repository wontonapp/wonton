package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Reply: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Reply",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f,
        autoMirror = true
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 200.0 32.0 V 176.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 H 67.31 l 34.35 34.34 a 8.0 8.0 0.0 0 1 -11.32 11.32 l -48.0 -48.0 a 8.0 8.0 0.0 0 1 0.0 -11.32 l 48.0 -48.0 a 8.0 8.0 0.0 0 1 11.32 11.32 L 67.31 168.0 H 184.0 V 32.0 a 8.0 8.0 0.0 0 1 16.0 0.0 Z")
        )
    }.build()
}
