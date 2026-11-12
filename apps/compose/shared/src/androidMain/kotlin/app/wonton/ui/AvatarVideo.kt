@file:androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)

package app.wonton.ui

import android.content.Context
import android.graphics.Bitmap
import android.media.MediaMetadataRetriever
import android.net.Uri
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.effect.Presentation
import androidx.media3.transformer.Composition
import androidx.media3.transformer.DefaultEncoderFactory
import androidx.media3.transformer.EditedMediaItem
import androidx.media3.transformer.Effects
import androidx.media3.transformer.ExportException
import androidx.media3.transformer.ExportResult
import androidx.media3.transformer.ProgressHolder
import androidx.media3.transformer.Transformer
import androidx.media3.transformer.VideoEncoderSettings
import app.wonton.client.ClientCore
import app.wonton.protocol.AvatarUpload
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

/** 视频头像的边长：规格定的转码目标。 */
private const val VIDEO_EDGE = 480

/** 视频首帧 JPEG 的质量（比正片的略低一档）。 */
private const val POSTER_QUALITY = 80

/** 视频转码从 1.5 Mbps 起，超过大小上限就按 [BITRATE_KEEP]/[BITRATE_DROP] 降码率再转一次（规格第 2 节）。 */
private const val BITRATE_START_BPS = 1_500_000
private const val BITRATE_FLOOR_BPS = 400_000
private const val BITRATE_KEEP = 2
private const val BITRATE_DROP = 3

private const val POLL_PROGRESS_MS = 100L

/** Transformer 的进度是 0 到 100 的整数。 */
private const val TRANSFORMER_PROGRESS_MAX = 100f

/**
 * 状态头像的视频（规格第 2 节）：居中裁方、缩到 [VIDEO_EDGE]、截前几秒、去声音、H.264 MP4，超过大小上限就降码率
 * 重转，并取首帧当海报。上限问核心（[ClientCore.maxAvatarBytes] 等），壳不写死数字。
 */
internal suspend fun video(context: Context, uri: Uri, core: ClientCore, onProgress: (Float) -> Unit): AvatarPick =
    withContext(Dispatchers.Default) {
        val (durationMs, side) = videoInfo(context, uri)
        val target = minOf(VIDEO_EDGE, side).let { it - (it % 2) }
        var bitrate = BITRATE_START_BPS
        val output = File.createTempFile("avatar-", ".mp4", context.cacheDir)
        try {
            while (true) {
                transcode(context, uri, output, durationMs, target, bitrate, core, onProgress)
                if (output.length() <= core.maxAvatarBytes().toLong() || bitrate <= BITRATE_FLOOR_BPS) break
                bitrate = bitrate * BITRATE_KEEP / BITRATE_DROP
                onProgress(0f)
            }
            val data = output.readBytes()
            AvatarPick.Ready(AvatarUpload("video/mp4", data, poster(output)))
        } finally {
            output.delete()
        }
    }

private inline fun <T> retriever(context: Context, uri: Uri, block: (MediaMetadataRetriever) -> T): T =
    MediaMetadataRetriever().use { retriever ->
        retriever.setDataSource(context, uri)
        block(retriever)
    }

private fun videoInfo(context: Context, uri: Uri): Pair<Long, Int> =
    retriever(context, uri) { retriever ->
        val duration = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull()
        val width = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)?.toIntOrNull() ?: 0
        val height = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)?.toIntOrNull() ?: 0
        val side = maxOf(width, height)
        if (duration == null || side == 0) throw IOException("Can't read this video")
        duration to side
    }

private suspend fun transcode(
    context: Context,
    uri: Uri,
    output: File,
    durationMs: Long,
    target: Int,
    bitrate: Int,
    core: ClientCore,
    onProgress: (Float) -> Unit,
) {
    val maxDuration = core.maxAvatarDurationMs().toLong()
    val clip = if (durationMs > maxDuration) {
        MediaItem.ClippingConfiguration.Builder().setEndPositionMs(maxDuration).build()
    } else {
        MediaItem.ClippingConfiguration.UNSET
    }
    val mediaItem = MediaItem.Builder().setUri(uri).setClippingConfiguration(clip).build()
    // SCALE_TO_FIT_WITH_CROP 到正方形目标：居中裁掉长边多出来的部分，再缩到 target（规格第 2 节）。
    val effects = Effects(
        emptyList(),
        listOf(Presentation.createForWidthAndHeight(target, target, Presentation.LAYOUT_SCALE_TO_FIT_WITH_CROP)),
    )
    val edited = EditedMediaItem.Builder(mediaItem).setRemoveAudio(true).setEffects(effects).build()
    val encoderFactory = DefaultEncoderFactory.Builder(context)
        .setRequestedVideoEncoderSettings(VideoEncoderSettings.Builder().setBitrate(bitrate).build())
        .build()
    val done = CompletableDeferred<Unit>()
    val transformer = Transformer.Builder(context)
        .setVideoMimeType(MimeTypes.VIDEO_H264)
        .setEncoderFactory(encoderFactory)
        .addListener(
            object : Transformer.Listener {
                override fun onCompleted(composition: Composition, exportResult: ExportResult) {
                    done.complete(Unit)
                }

                override fun onError(
                    composition: Composition,
                    exportResult: ExportResult,
                    exportException: ExportException,
                ) {
                    done.completeExceptionally(exportException)
                }
            },
        )
        .build()
    try {
        transformer.start(edited, output.path)
        val holder = ProgressHolder()
        while (transformer.getProgress(holder) == Transformer.PROGRESS_STATE_AVAILABLE && !done.isCompleted) {
            onProgress(holder.progress / TRANSFORMER_PROGRESS_MAX)
            delay(POLL_PROGRESS_MS)
        }
        done.await()
        onProgress(1f)
    } catch (cancellation: CancellationException) {
        runCatching { transformer.cancel() }
        throw cancellation
    } catch (failure: ExportException) {
        runCatching { transformer.cancel() }
        throw failure
    }
}

/** 视频首帧存成 JPEG 当海报（规格第 2 节）；取不出就给 null，核心收得下没有海报的视频。 */
@Suppress("SwallowedException")
private fun poster(file: File): ByteArray? = try {
    MediaMetadataRetriever().use { retriever ->
        retriever.setDataSource(file.path)
        val frame = retriever.getFrameAtTime(0, MediaMetadataRetriever.OPTION_CLOSEST_SYNC) ?: return null
        val out = ByteArrayOutputStream()
        if (!frame.compress(Bitmap.CompressFormat.JPEG, POSTER_QUALITY, out)) return null
        frame.recycle()
        out.toByteArray()
    }
} catch (failure: IOException) {
    null
} catch (failure: IllegalStateException) {
    null
}
