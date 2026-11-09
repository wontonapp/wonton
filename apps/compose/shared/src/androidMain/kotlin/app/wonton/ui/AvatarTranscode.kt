package app.wonton.ui

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.graphics.drawable.AnimatedImageDrawable
import android.net.Uri
import app.wonton.client.ClientCore
import app.wonton.protocol.AvatarUpload
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.nio.ByteBuffer

/** 图片头像的边长：规格定的转码目标，比核心的上限小，转出来核心一定收得下。 */
private const val IMAGE_EDGE = 512

/** 输出 JPEG 的质量。 */
private const val JPEG_QUALITY = 85

/** 主人选中并转好的头像：[Ready] 交给核心，[UseVideo] 提示动图太大、改用视频（规格第 2 节）。 */
internal sealed interface AvatarPick {
    data class Ready(val upload: AvatarUpload) : AvatarPick

    data object UseVideo : AvatarPick
}

/**
 * 把主人选的一个文件转成核心收得下的头像（规格第 2 节）：图片居中裁方、缩到 [IMAGE_EDGE]、JPEG（PNG 保透明）；
 * 动图原样交（方形、在核心的上限内），否则状态头像提示用视频。上限问核心（[ClientCore.maxAvatarEdge] 等），
 * 壳不写死数字。视频在 [video] 里。
 */
internal object AvatarTranscode {
    suspend fun make(
        context: Context,
        uri: Uri,
        core: ClientCore,
        forState: Boolean,
        onProgress: (Float) -> Unit,
    ): AvatarPick {
        val type = context.contentResolver.getType(uri)
        return when {
            type == "image/gif" || type == "image/webp" -> animated(context, uri, core, forState)
            type?.startsWith("video/") == true && forState -> video(context, uri, core, onProgress)
            type?.startsWith("image/") == true -> still(context, uri)
            else -> throw IOException("Unrecognized file: $type")
        }
    }

    /** 动图原样交；形状或大小不行时，状态头像改用视频，静态头像取首帧当静态图。 */
    private suspend fun animated(context: Context, uri: Uri, core: ClientCore, forState: Boolean): AvatarPick {
        val data = context.contentResolver.openInputStream(uri)?.use { it.readBytes() }
            ?: throw IOException("Can't read the picked file")
        val mime = context.contentResolver.getType(uri)!!
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(data, 0, data.size, bounds)
        val square = bounds.outWidth > 0 && bounds.outWidth == bounds.outHeight
        val fits = square &&
            bounds.outWidth <= core.maxAvatarEdge().toInt() &&
            data.size.toLong() <= core.maxAvatarBytes().toLong() &&
            isAnimated(data)
        return when {
            fits -> AvatarPick.Ready(AvatarUpload(mime, data, null))
            forState -> AvatarPick.UseVideo
            // 静态头像没有视频可选：按静态图转（首帧）。
            else -> still(context, uri)
        }
    }

    @Suppress("SwallowedException")
    private fun isAnimated(data: ByteArray): Boolean = try {
        ImageDecoder.decodeDrawable(ImageDecoder.createSource(ByteBuffer.wrap(data))) is AnimatedImageDrawable
    } catch (failure: IOException) {
        false
    } catch (failure: IllegalStateException) {
        false
    }

    /** 图片：居中裁成正方形，只缩不放（不超过 [IMAGE_EDGE]），PNG 有透明输出 PNG，其余 JPEG。 */
    private fun still(context: Context, uri: Uri): AvatarPick {
        val data = context.contentResolver.openInputStream(uri)?.use { it.readBytes() }
            ?: throw IOException("Can't read the picked file")
        val bitmap = decode(data)
        val side = minOf(bitmap.width, bitmap.height)
        val cropped = Bitmap.createBitmap(bitmap, (bitmap.width - side) / 2, (bitmap.height - side) / 2, side, side)
        val scaled = if (side > IMAGE_EDGE) {
            Bitmap.createScaledBitmap(cropped, IMAGE_EDGE, IMAGE_EDGE, true)
        } else {
            cropped
        }
        val png = context.contentResolver.getType(uri) == "image/png" && scaled.hasAlpha()
        val format = if (png) Bitmap.CompressFormat.PNG else Bitmap.CompressFormat.JPEG
        val mime = if (png) "image/png" else "image/jpeg"
        return AvatarPick.Ready(AvatarUpload(mime, encode(scaled, format, JPEG_QUALITY), null))
    }

    /** 解码时按 EXIF 方向转正（BitmapFactory 不会），先按采样缩小，照片太大也不吃内存。 */
    private fun decode(data: ByteArray): Bitmap =
        ImageDecoder.decodeBitmap(ImageDecoder.createSource(ByteBuffer.wrap(data))) { decoder, info, _ ->
            val longer = maxOf(info.size.width, info.size.height)
            var sample = 1
            while (longer / (sample * 2) >= IMAGE_EDGE) sample *= 2
            decoder.setTargetSampleSize(sample)
        } ?: throw IOException("Can't decode this image")

    private fun encode(bitmap: Bitmap, format: Bitmap.CompressFormat, quality: Int): ByteArray {
        val out = ByteArrayOutputStream()
        if (!bitmap.compress(format, quality, out)) throw IOException("Can't encode this image")
        return out.toByteArray()
    }
}
