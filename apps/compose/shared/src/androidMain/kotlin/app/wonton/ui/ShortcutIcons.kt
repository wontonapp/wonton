package app.wonton.ui

import android.graphics.Bitmap
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.CanvasDrawScope
import androidx.compose.ui.graphics.drawscope.inset
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.rememberVectorPainter
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.LayoutDirection
import androidx.core.graphics.drawable.IconCompat

private const val SHORTCUT_ICON_SIZE = 192
private const val SHORTCUT_ICON_INSET = 48f

@Composable
internal fun rememberShortcutIcon(vector: ImageVector): IconCompat {
    val painter = rememberVectorPainter(vector)
    val density = LocalDensity.current
    return remember(painter, density) {
        val bitmap = Bitmap.createBitmap(SHORTCUT_ICON_SIZE, SHORTCUT_ICON_SIZE, Bitmap.Config.ARGB_8888)
        CanvasDrawScope().draw(
            density, LayoutDirection.Ltr, Canvas(bitmap.asImageBitmap()),
            Size(SHORTCUT_ICON_SIZE.toFloat(), SHORTCUT_ICON_SIZE.toFloat()),
        ) {
            drawRect(Color.White)
            inset(SHORTCUT_ICON_INSET) { with(painter) { draw(size) } }
        }
        IconCompat.createWithBitmap(bitmap)
    }
}
