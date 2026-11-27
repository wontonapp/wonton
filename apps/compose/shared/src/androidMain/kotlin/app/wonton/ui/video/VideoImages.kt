package app.wonton.ui.video

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.LruCache
import androidx.compose.animation.Crossfade
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.net.HttpURLConnection
import java.net.URI
import java.security.MessageDigest

/** 内存里解码好的图最多占这么多字节：海报墙一屏几十张，每张解码后几 MB，只数张数挡不住内存。 */
private const val MEMORY_BYTES = 120 * 1024 * 1024
/** 磁盘上原字节的上限，超了按最久没用的删到八成。 */
private const val DISK_BYTES = 300L * 1024 * 1024
private const val DISK_KEEP_PERCENT = 80
private const val PERCENT = 100
private const val TRIM_EVERY = 50
/** 解码时长边最多这么多像素：背景大图也不用比屏幕更细。 */
private const val MAX_EDGE = 1600
private const val TIMEOUT_MS = 15_000
private const val HTTP_OK = 200

/** 图片缓存的作用域（`video_cache_scope`），影视页的根设一次。 */
val LocalVideoCacheScope = compositionLocalOf { "" }

/**
 * 海报、剧照、背景图的加载：字节走客户端核的本机回环。回环的端口和令牌每次启动都换，缓存键去掉它们
 * （地址里第一段路径之后的部分），App 重启后不用重取；换了服务器或账号同一路径是另一张图，键前面带上
 * 客户端核给的作用域。没用图片库：地址只有本机回环一种，要的只是换掉缓存键，几十行能写清楚，不值得多带一个库。
 */
internal object VideoImages {
    private val memory = object : LruCache<String, Bitmap>(MEMORY_BYTES) {
        override fun sizeOf(key: String, value: Bitmap): Int = value.allocationByteCount
    }
    private var writes = 0

    /** 回环地址 `http://127.0.0.1:<端口>/<令牌>/items/…` 在 `scope` 下的缓存键：作用域加 `/items/…` 连同查询串。 */
    fun key(url: String, scope: String): String {
        // 按字符串切，不用 URI 解析：地址是核心给的，认不得的字符也只该让这张图取不到，不该让页面崩。
        val path = url.substringAfter("://").substringAfter('/', "")
        return "$scope\n" + path.substringAfter('/', path)
    }

    /** 取不到（没有这张、断了）就是空：图是装饰，卡片留占位，不单独报错。 */
    suspend fun image(context: Context, url: String, scope: String): Bitmap? {
        val key = key(url, scope)
        memory.get(key)?.let { return it }
        return withContext(Dispatchers.IO) {
            val file = File(directory(context), digest(key))
            val bytes = if (file.exists()) {
                file.setLastModified(System.currentTimeMillis())
                file.readBytes()
            } else {
                fetch(url)?.also { save(context, file, it) }
            }
            bytes?.let(::decode)?.also { memory.put(key, it) }
        }
    }

    private fun fetch(url: String): ByteArray? = runCatching {
        val connection = URI(url).toURL().openConnection() as HttpURLConnection
        connection.connectTimeout = TIMEOUT_MS
        connection.readTimeout = TIMEOUT_MS
        try {
            if (connection.responseCode == HTTP_OK) connection.inputStream.use { it.readBytes() } else null
        } finally {
            connection.disconnect()
        }
    }.getOrNull()

    /** 按长边缩到 [MAX_EDGE] 以内解码。 */
    private fun decode(bytes: ByteArray): Bitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        var sample = 1
        while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= MAX_EDGE) sample *= 2
        val options = BitmapFactory.Options().apply { inSampleSize = sample }
        return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
    }

    private fun save(context: Context, file: File, bytes: ByteArray) {
        val temporary = File(file.parentFile, "${file.name}.tmp")
        runCatching {
            temporary.writeBytes(bytes)
            temporary.renameTo(file)
        }
        synchronized(this) {
            writes += 1
            if (writes % TRIM_EVERY == 0) trim(directory(context))
        }
    }

    /** 超了上限按修改时间（取用时会刷新）从旧往新删，删到上限的八成。 */
    private fun trim(directory: File) {
        val files = directory.listFiles()?.sortedBy { it.lastModified() } ?: return
        var total = files.sumOf { it.length() }
        if (total <= DISK_BYTES) return
        for (file in files) {
            if (total <= DISK_BYTES * DISK_KEEP_PERCENT / PERCENT) break
            total -= file.length()
            file.delete()
        }
    }

    private fun directory(context: Context): File =
        File(context.cacheDir, "video-artwork").also { it.mkdirs() }

    private fun digest(key: String): String =
        MessageDigest.getInstance("SHA-256").digest(key.toByteArray()).joinToString("") { "%02x".format(it) }
}

/**
 * 一张图：没有地址或取不到时是占位色块，上面可以放 [placeholder]（如片名）。图铺满给它的框、裁掉多出的部分；
 * 台标这类要整张露出来的传 `fit`，没有底色。
 */
@Composable
fun VideoArtwork(
    url: String?,
    modifier: Modifier = Modifier,
    fit: Boolean = false,
    placeholder: @Composable BoxScope.() -> Unit = {},
) {
    val context = LocalContext.current
    val scope = LocalVideoCacheScope.current
    val image by produceState<Bitmap?>(null, url, scope) {
        value = url?.let { VideoImages.image(context, it, scope) }
    }
    val base = if (fit) modifier else modifier.background(WontonTheme.colors.surfaceSubtle.toColor())
    Box(base.clipToBounds()) {
        Crossfade(image, label = "artwork") { bitmap ->
            if (bitmap != null) {
                Image(
                    bitmap.asImageBitmap(),
                    // 装饰：卡片和标题已经说了是哪一部。
                    contentDescription = null,
                    Modifier.fillMaxSize(),
                    contentScale = if (fit) ContentScale.Fit else ContentScale.Crop,
                )
            } else {
                Box(Modifier.fillMaxSize(), content = placeholder)
            }
        }
    }
}
