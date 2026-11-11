package app.wonton.ui

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.net.Uri
import app.wonton.protocol.ImageThumbnail
import app.wonton.protocol.ImageUpload
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.nio.ByteBuffer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** 尺寸上限、转码和缩略图的规则见 ADR-0005 的“App 内聊天的界面”。 */
object PhotoUpload {
    private const val MAX_SIDE = 4096
    private const val THUMBNAIL_SIZE = 800
    private const val PHOTO_QUALITY = 90
    private const val THUMBNAIL_QUALITY = 80

    // PNG 无损，Bitmap.compress 不看这个值。
    private const val LOSSLESS = 100

    suspend fun make(context: Context, uri: Uri): Result<ImageUpload> = withContext(Dispatchers.Default) {
        try {
            val resolver = context.contentResolver
            val data = resolver.openInputStream(uri)?.use { it.readBytes() }
                ?: throw IOException(context.getString(R.string.photo_unreadable))
            val type = resolver.getType(uri)
            val upload = if (type == "image/gif") {
                gif(context, data)
            } else {
                reencode(context, data, png = type == "image/png")
            }
            Result.success(upload)
        } catch (failure: IOException) {
            Result.failure(failure)
        }
    }

    /** 原样发，保留动画；尺寸取自文件头，不解码。 */
    private fun gif(context: Context, data: ByteArray): ImageUpload {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(data, 0, data.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) {
            throw IOException(context.getString(R.string.photo_gif_unreadable))
        }
        val thumbnail = thumbnail(context, data, longerSide = maxOf(bounds.outWidth, bounds.outHeight))
        return ImageUpload(
            "photo.gif",
            "image/gif",
            data,
            bounds.outWidth.toUInt(),
            bounds.outHeight.toUInt(),
            thumbnail,
        )
    }

    private fun reencode(context: Context, data: ByteArray, png: Boolean): ImageUpload {
        val image = decode(data) { width, height -> setTargetSampleSize(sampleSize(maxOf(width, height))) }
        val thumbnail = thumbnail(context, data, longerSide = maxOf(image.width, image.height))
        val encoded = if (png) {
            encode(context, image, Bitmap.CompressFormat.PNG, LOSSLESS)
        } else {
            encode(context, image, Bitmap.CompressFormat.JPEG, PHOTO_QUALITY)
        }
        val upload = ImageUpload(
            if (png) "photo.png" else "photo.jpg",
            if (png) "image/png" else "image/jpeg",
            encoded,
            image.width.toUInt(),
            image.height.toUInt(),
            thumbnail,
        )
        image.recycle()
        return upload
    }

    private fun thumbnail(context: Context, data: ByteArray, longerSide: Int): ImageThumbnail? {
        if (longerSide <= THUMBNAIL_SIZE) return null
        // 缩略图直接给目标尺寸：Skia 先解出的那一档长边不到 1600，用不着按 1/2ⁿ 缩。
        val small = decode(data) { width, height ->
            val scale = THUMBNAIL_SIZE.toFloat() / maxOf(width, height)
            setTargetSize((width * scale).toInt().coerceAtLeast(1), (height * scale).toInt().coerceAtLeast(1))
        }
        val thumbnail = ImageThumbnail(
            "image/jpeg",
            encode(context, small, Bitmap.CompressFormat.JPEG, THUMBNAIL_QUALITY),
            small.width.toUInt(),
            small.height.toUInt(),
        )
        small.recycle()
        return thumbnail
    }

    /** `ImageDecoder` 解码时按 EXIF 方向转正，动图只取第一帧；`resize` 拿到原图的宽高，决定解成多大。 */
    private fun decode(data: ByteArray, resize: ImageDecoder.(width: Int, height: Int) -> Unit): Bitmap =
        ImageDecoder.decodeBitmap(ImageDecoder.createSource(ByteBuffer.wrap(data))) { decoder, info, _ ->
            // 要在 CPU 上读像素再编码，不能用硬件位图。
            decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
            decoder.resize(info.size.width, info.size.height)
        }

    /**
     * 主图按 1/2、1/4… 缩到长边不超过 `MAX_SIDE`，解码器按这个采样倍数直接解出目标尺寸。用 `setTargetSize` 缩到别的
     * 尺寸时，Skia 先解出不小于目标的最近一档再缩：长边 4096 到 8192 的图（4800 万、5000 万像素的照片）要先解出原尺寸，
     * 将近 200 MB。
     */
    private fun sampleSize(longerSide: Int): Int {
        var sample = 1
        while ((longerSide + sample - 1) / sample > MAX_SIDE) sample *= 2
        return sample
    }

    /** 只写像素：原图的 EXIF、GPS、XMP、IPTC 都不会写进去。 */
    private fun encode(context: Context, bitmap: Bitmap, format: Bitmap.CompressFormat, quality: Int): ByteArray =
        ByteArrayOutputStream().use { out ->
            if (!bitmap.compress(format, quality, out)) throw IOException(context.getString(R.string.photo_unencodable))
            out.toByteArray()
        }
}
