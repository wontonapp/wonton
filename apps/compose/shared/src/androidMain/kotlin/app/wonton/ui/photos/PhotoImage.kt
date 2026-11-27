package app.wonton.ui.photos

import android.content.Context
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.layout.ContentScale
import app.wonton.client.ClientCore
import app.wonton.protocol.PhotoRendition
import coil3.ImageLoader
import coil3.compose.AsyncImage
import coil3.compose.AsyncImagePainter
import coil3.compose.LocalPlatformContext
import coil3.decode.DataSource
import coil3.decode.ImageSource
import coil3.disk.DiskCache
import coil3.fetch.FetchResult
import coil3.fetch.Fetcher
import coil3.fetch.SourceFetchResult
import coil3.key.Keyer
import coil3.memory.MemoryCache
import coil3.request.ImageRequest
import coil3.request.crossfade
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okio.Buffer
import okio.FileSystem
import okio.Path.Companion.toOkioPath
import okio.source
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL

private const val DISK_CACHE_BYTES = 512L * 1024 * 1024
private const val MEMORY_CACHE_PERCENT = 0.2
private const val CONNECT_TIMEOUT_MS = 10_000
private const val READ_TIMEOUT_MS = 60_000

/**
 * 一张图的一档，或一个人物的头像（[rendition] 为空）。缓存按 [cacheKey] 存：条目 id 加档位，再带上服务器和相册应用
 * （两处的 id 可能撞）。核心给的回环地址每次启动都换端口和令牌，不能当键。
 */
internal data class PhotoRef(
    val account: String?,
    val source: String?,
    val id: String,
    val rendition: PhotoRendition?,
) {
    val cacheKey: String
        get() = listOf(account.orEmpty(), source.orEmpty(), id, rendition?.name ?: "person").joinToString("|")
}

/** 相册页用的图片加载器（Coil）；由 [PhotosRoute] 按核心实例提供。 */
internal val LocalPhotoLoader = compositionLocalOf<ImageLoader> { error("photo loader is not provided") }

private object PhotoDiskCache {
    @Volatile
    private var cache: DiskCache? = null

    /** 整个进程一份：同一个目录不能开两个缓存，换服务器重建加载器时接着用它。 */
    fun get(context: Context): DiskCache = cache ?: synchronized(this) {
        cache ?: DiskCache.Builder()
            .directory(context.cacheDir.resolve("photo-library").toOkioPath())
            .maxSizeBytes(DISK_CACHE_BYTES)
            .build()
            .also { cache = it }
    }
}

internal fun photoLoader(context: Context, core: ClientCore): ImageLoader {
    val app = context.applicationContext
    return ImageLoader.Builder(app)
        .components {
            add(Keyer<PhotoRef> { data, _ -> data.cacheKey })
            add(Fetcher.Factory<PhotoRef> { data, _, loader -> PhotoFetcher(data, core, loader) })
        }
        .memoryCache { MemoryCache.Builder().maxSizePercent(app, MEMORY_CACHE_PERCENT).build() }
        .diskCache(PhotoDiskCache.get(app))
        .build()
}

/** 先看磁盘缓存；没有才向核心要这次启动的回环地址，取回来的字节按 [PhotoRef.cacheKey] 落盘。 */
private class PhotoFetcher(
    private val ref: PhotoRef,
    private val core: ClientCore,
    private val loader: ImageLoader,
) : Fetcher {
    override suspend fun fetch(): FetchResult {
        val cache = loader.diskCache
        cache?.openSnapshot(ref.cacheKey)?.let { snapshot ->
            return SourceFetchResult(
                ImageSource(snapshot.data, cache.fileSystem, ref.cacheKey, snapshot),
                mimeType = null,
                dataSource = DataSource.DISK,
            )
        }
        val url = when (val rendition = ref.rendition) {
            null -> core.photoPersonUrl(ref.id)
            else -> core.photoUrl(ref.id, rendition)
        }
        return withContext(Dispatchers.IO) { download(url, cache) }
    }

    private fun download(url: String, cache: DiskCache?): FetchResult {
        val connection = URL(url).openConnection() as HttpURLConnection
        connection.connectTimeout = CONNECT_TIMEOUT_MS
        connection.readTimeout = READ_TIMEOUT_MS
        try {
            val status = connection.responseCode
            if (status != HttpURLConnection.HTTP_OK) throw IOException("HTTP $status")
            val mime = connection.contentType?.substringBefore(';')?.trim()
            val editor = cache?.openEditor(ref.cacheKey)
            if (cache == null || editor == null) {
                val bytes = Buffer().apply { connection.inputStream.source().use { writeAll(it) } }
                return SourceFetchResult(ImageSource(bytes, FileSystem.SYSTEM), mime, DataSource.NETWORK)
            }
            var written = false
            try {
                cache.fileSystem.write(editor.data) { connection.inputStream.source().use { writeAll(it) } }
                written = true
            } finally {
                if (!written) editor.abort()
            }
            val snapshot = editor.commitAndOpenSnapshot() ?: throw IOException("photo cache entry vanished")
            val source = ImageSource(snapshot.data, cache.fileSystem, ref.cacheKey, snapshot)
            return SourceFetchResult(source, mime, DataSource.NETWORK)
        } finally {
            connection.disconnect()
        }
    }
}

/**
 * 一张图：底下是 ThumbHash 占位，字节到了盖上去。取不到时占位留着，[onState] 把结果交给要说明原因的页面（大图页）。
 * [retry] 变了就重取一次。
 */
@Composable
internal fun PhotoImage(
    ref: PhotoRef,
    thumbhash: String?,
    label: String?,
    modifier: Modifier = Modifier,
    scale: ContentScale = ContentScale.Crop,
    retry: Int = 0,
    onState: ((AsyncImagePainter.State) -> Unit)? = null,
) {
    val context = LocalPlatformContext.current
    val request = remember(ref, retry) {
        ImageRequest.Builder(context).data(ref).crossfade(true).build()
    }
    Box(modifier) {
        rememberThumbHash(thumbhash)?.let { placeholder ->
            // 装饰：真图的读屏名在上面那层。
            Image(
                placeholder,
                contentDescription = null,
                Modifier.matchParentSize(),
                contentScale = scale,
                filterQuality = FilterQuality.Low,
            )
        }
        AsyncImage(
            model = request,
            imageLoader = LocalPhotoLoader.current,
            contentDescription = label,
            modifier = Modifier.matchParentSize(),
            contentScale = scale,
            onState = onState,
        )
    }
}
