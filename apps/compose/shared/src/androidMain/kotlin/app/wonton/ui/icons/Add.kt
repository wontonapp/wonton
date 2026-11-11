package app.wonton.ui.icons

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

@Suppress("UnusedReceiverParameter")
val WontonIcon.Add: ImageVector by lazy(LazyThreadSafetyMode.NONE) {
    ImageVector.Builder(
        name = "Add",
        defaultWidth = 256.dp,
        defaultHeight = 256.dp,
        viewportWidth = 256f,
        viewportHeight = 256f
    ).apply {
        addPath(
            fill = SolidColor(Color.Black),
            pathData = addPathNodes("M 224.0 128.0 a 8.0 8.0 0.0 0 1 -8.0 8.0 H 136.0 v 80.0 a 8.0 8.0 0.0 0 1 -16.0 0.0 V 136.0 H 40.0 a 8.0 8.0 0.0 0 1 0.0 -16.0 h 80.0 V 40.0 a 8.0 8.0 0.0 0 1 16.0 0.0 v 80.0 h 80.0 A 8.0 8.0 0.0 0 1 224.0 128.0 Z")
        )
    }.build()
}
