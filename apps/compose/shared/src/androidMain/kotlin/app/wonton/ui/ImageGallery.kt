package app.wonton.ui

import android.content.ContentResolver
import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import android.webkit.MimeTypeMap
import java.io.File
import java.io.IOException

/**
 * 把聊天图片的字节存进系统相册（Pictures/Wonton），返回新条目的 URI。
 * 本 App 的 minSdk 是 29，MediaStore 的这个写法在所有支持的版本上都不需要存储权限。
 */
fun saveImageToGallery(context: Context, bytes: ByteArray, filename: String, declaredMime: String?): Uri {
    val resolver = context.contentResolver
    val mime = imageMime(declaredMime, bytes, filename)
    val pending = ContentValues().apply {
        put(MediaStore.MediaColumns.DISPLAY_NAME, galleryName(filename, mime))
        mime?.let { put(MediaStore.MediaColumns.MIME_TYPE, it) }
        put(MediaStore.MediaColumns.RELATIVE_PATH, Environment.DIRECTORY_PICTURES + File.separator + "Wonton")
        put(MediaStore.MediaColumns.IS_PENDING, 1)
    }
    val uri = resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, pending)
        ?: throw IOException("MediaStore rejected the new gallery entry")
    writeGalleryEntry(resolver, uri, bytes)
    resolver.update(uri, ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }, null, null)
    return uri
}

/** 写入并发布相册条目；写失败时清掉条目，不留写了一半的。 */
// 失败原因各不相同（磁盘满、权限、条目失效），都如实交回给调用方；这里只负责清干净。
@Suppress("TooGenericExceptionCaught")
private fun writeGalleryEntry(resolver: ContentResolver, uri: Uri, bytes: ByteArray) {
    try {
        resolver.openOutputStream(uri)?.use { it.write(bytes) }
            ?: throw IOException("Could not open the new gallery entry for writing")
    } catch (failure: Exception) {
        resolver.delete(uri, null, null)
        throw failure
    }
}

/**
 * 把图片字节落进 App 私有缓存，返回交给系统分享面板的（文件路径、MIME）。
 * 经 FileProvider 给出去，不放外部公开目录；写不了缓存文件时抛 [IOException]。
 */
fun shareImageFile(
    context: Context,
    bytes: ByteArray,
    filename: String,
    declaredMime: String?,
): Pair<String, String?> {
    val dir = File(context.cacheDir, "share").apply { mkdirs() }
    // 缓存里只留正在分享的这一份。
    dir.listFiles()?.forEach { it.delete() }
    val mime = imageMime(declaredMime, bytes, filename)
    val file = File(dir, galleryName(filename, mime))
    file.writeBytes(bytes)
    return file.path to mime
}

/** 发送方声明的 MIME 先用（只认 image/ 开头的），再用文件头认，最后按扩展名猜。 */
fun imageMime(declared: String?, bytes: ByteArray, filename: String): String? =
    declared?.takeIf { it.startsWith("image/") }
        ?: sniffImageMime(bytes)
        ?: MimeTypeMap.getSingleton()
            .getMimeTypeFromExtension(filename.substringAfterLast('.', "").lowercase())
            ?.takeIf { it.startsWith("image/") }

/** 按文件头认常见图片格式；认不出就交回 null。 */
fun sniffImageMime(bytes: ByteArray): String? = when {
    bytes.has(JPEG_SIGNATURE) -> "image/jpeg"
    bytes.has(PNG_SIGNATURE) -> "image/png"
    bytes.has(RIFF_SIGNATURE) && bytes.hasAt(RIFF_TYPE_OFFSET, WEBP_SIGNATURE) -> "image/webp"
    bytes.has(GIF_SIGNATURE) -> "image/gif"
    bytes.has(BMP_SIGNATURE) -> "image/bmp"
    else -> null
}

// 常见图片格式的文件头。
private val JPEG_SIGNATURE = intArrayOf(0xFF, 0xD8)
private val PNG_SIGNATURE = intArrayOf(0x89, 0x50, 0x4E, 0x47)
private val RIFF_SIGNATURE = intArrayOf(0x52, 0x49, 0x46, 0x46)
private const val RIFF_TYPE_OFFSET = 8
private val WEBP_SIGNATURE = intArrayOf(0x57, 0x45, 0x42, 0x50)
private val GIF_SIGNATURE = intArrayOf(0x47, 0x49, 0x46)
private val BMP_SIGNATURE = intArrayOf(0x42, 0x4D)

/** 相册条目和缓存文件的名字：只留最后一个路径段和文件名里合法的字符，没有扩展名时按 MIME 补上。 */
fun galleryName(filename: String, mime: String?): String {
    val base = filename.substringAfterLast('/').substringAfterLast('\\')
        .replace(ILLEGAL_NAME_CHARS, "").trim().ifEmpty { "wonton-image" }
    val extension = mime?.let { MimeTypeMap.getSingleton().getExtensionFromMimeType(it) }
    return when {
        base.contains('.') -> base
        extension == null -> base
        else -> "$base.$extension"
    }
}

private val ILLEGAL_NAME_CHARS = Regex("[\\\\/:*?\"<>|\\p{Cntrl}]")

private fun ByteArray.has(prefix: IntArray): Boolean =
    size >= prefix.size && prefix.indices.all { this[it] == prefix[it].toByte() }

private fun ByteArray.hasAt(offset: Int, prefix: IntArray): Boolean =
    size >= offset + prefix.size && prefix.indices.all { this[offset + it] == prefix[it].toByte() }
