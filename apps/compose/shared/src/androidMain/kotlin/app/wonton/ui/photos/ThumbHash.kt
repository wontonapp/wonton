// 位布局和系数是 ThumbHash 格式自己定的，起名字反而对不上规范。
@file:Suppress("MagicNumber")

package app.wonton.ui.photos

import android.graphics.Bitmap
import android.util.Base64
import android.util.LruCache
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.max
import kotlin.math.roundToInt

private const val CACHED_PLACEHOLDERS = 512
private const val LONG_SIDE = 32

private val placeholders = LruCache<String, ImageBitmap>(CACHED_PLACEHOLDERS)

/** ThumbHash（base64）解成一张 32 像素以内的小图，放大了当占位；解不出就没有占位。 */
@Composable
internal fun rememberThumbHash(hash: String?): ImageBitmap? = remember(hash) {
    if (hash == null) return@remember null
    placeholders.get(hash) ?: decodeThumbHash(hash)?.also { placeholders.put(hash, it) }
}

private fun decodeThumbHash(encoded: String): ImageBitmap? = try {
    val hash = Base64.decode(encoded, Base64.DEFAULT)
    if (hash.size < 5) null else ThumbHash(hash).render()
} catch (_: IllegalArgumentException) {
    null
} catch (_: IndexOutOfBoundsException) {
    null
}

/**
 * ThumbHash 的解码（格式见 https://evanw.github.io/thumbhash/ ，参考实现 MIT）：头 5–6 字节是亮度、两个色差和透明度的
 * 直流分量与缩放，后面每半字节一个交流系数，按离散余弦变换还原。
 */
private class ThumbHash(private val hash: ByteArray) {
    private val header24 = byte(0) or (byte(1) shl 8) or (byte(2) shl 16)
    private val header16 = byte(3) or (byte(4) shl 8)
    private val hasAlpha = header24 shr 23 != 0
    private val landscape = header16 shr 15 != 0
    private val lx = max(3, if (landscape) (if (hasAlpha) 5 else 7) else header16 and 7)
    private val ly = max(3, if (landscape) header16 and 7 else (if (hasAlpha) 5 else 7))
    private var cursor = 0

    private fun byte(index: Int) = hash[index].toInt() and 0xFF

    /** 一个通道的交流系数：三角形排布，跳过 (0,0) 的直流项。 */
    private fun channel(nx: Int, ny: Int, scale: Float): FloatArray {
        val start = if (hasAlpha) 6 else 5
        val values = ArrayList<Float>()
        for (cy in 0 until ny) {
            var cx = if (cy == 0) 1 else 0
            while (cx * ny < nx * (ny - cy)) {
                val nibble = (byte(start + (cursor shr 1)) shr ((cursor and 1) shl 2)) and 15
                cursor += 1
                values += (nibble / 7.5f - 1f) * scale
                cx += 1
            }
        }
        return values.toFloatArray()
    }

    /** 系数和余弦基相乘求和；[nx]、[ny] 同 [channel]。 */
    private fun sum(ac: FloatArray, nx: Int, ny: Int, fx: FloatArray, fy: FloatArray): Float {
        var total = 0f
        var index = 0
        for (cy in 0 until ny) {
            var cx = if (cy == 0) 1 else 0
            while (cx * ny < nx * (ny - cy)) {
                total += ac[index] * fx[cx] * fy[cy] * 2f
                index += 1
                cx += 1
            }
        }
        return total
    }

    fun render(): ImageBitmap {
        val lDc = (header24 and 63) / 63f
        val pDc = ((header24 shr 6) and 63) / 31.5f - 1f
        val qDc = ((header24 shr 12) and 63) / 31.5f - 1f
        val lScale = ((header24 shr 18) and 31) / 31f
        val pScale = ((header16 shr 3) and 63) / 63f
        val qScale = ((header16 shr 9) and 63) / 63f
        val aDc = if (hasAlpha) (byte(5) and 15) / 15f else 1f
        val aScale = if (hasAlpha) (byte(5) shr 4) / 15f else 0f
        val lAc = channel(lx, ly, lScale)
        // 色差乘 1.25：量化丢掉的饱和度补回来。
        val pAc = channel(3, 3, pScale * 1.25f)
        val qAc = channel(3, 3, qScale * 1.25f)
        val aAc = if (hasAlpha) channel(5, 5, aScale) else null
        val ratio = lx.toFloat() / ly
        val width = (if (ratio > 1f) LONG_SIDE.toFloat() else LONG_SIDE * ratio).roundToInt()
        val height = (if (ratio > 1f) LONG_SIDE / ratio else LONG_SIDE.toFloat()).roundToInt()
        val terms = max(max(lx, ly), if (hasAlpha) 5 else 3)
        val fx = FloatArray(terms)
        val fy = FloatArray(terms)
        val pixels = IntArray(width * height)
        for (y in 0 until height) {
            for (cy in 0 until terms) fy[cy] = cos(PI / height * (y + 0.5) * cy).toFloat()
            for (x in 0 until width) {
                for (cx in 0 until terms) fx[cx] = cos(PI / width * (x + 0.5) * cx).toFloat()
                val l = lDc + sum(lAc, lx, ly, fx, fy)
                val p = pDc + sum(pAc, 3, 3, fx, fy)
                val q = qDc + sum(qAc, 3, 3, fx, fy)
                val a = if (aAc != null) aDc + sum(aAc, 5, 5, fx, fy) else 1f
                val b = l - 2f / 3f * p
                val r = (3f * l - b + q) / 2f
                pixels[y * width + x] = argb(a, r, r - q, b)
            }
        }
        return Bitmap.createBitmap(pixels, width, height, Bitmap.Config.ARGB_8888).asImageBitmap()
    }

    private fun argb(a: Float, r: Float, g: Float, b: Float): Int {
        fun level(value: Float) = (255f * value.coerceIn(0f, 1f)).roundToInt()
        return (level(a) shl 24) or (level(r) shl 16) or (level(g) shl 8) or level(b)
    }
}
