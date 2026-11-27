package app.wonton.ui

import kotlin.math.abs

/** 一次按压往哪个方向走：判成竖向后就不再管，交给列表。 */
internal enum class DragAxis { Undecided, Horizontal, Vertical }

/** 走过触摸阈值才定方向；没到就是 Undecided。竖向略占先手，横竖一样长时算竖向。 */
internal fun axisAfter(dx: Float, dy: Float, slop: Float): DragAxis = when {
    abs(dx) <= slop && abs(dy) <= slop -> DragAxis.Undecided
    abs(dy) >= abs(dx) -> DragAxis.Vertical
    else -> DragAxis.Horizontal
}
