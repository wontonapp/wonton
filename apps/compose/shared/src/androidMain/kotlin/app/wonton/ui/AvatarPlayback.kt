package app.wonton.ui

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.graphics.ImageDecoder
import android.os.PowerManager
import android.provider.Settings
import android.util.LruCache
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import app.wonton.client.ClientCore
import app.wonton.protocol.CoreException
import app.wonton.protocol.MediaHandle
import java.io.IOException
import java.nio.ByteBuffer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.withContext

/** 一屏同时播放的状态头像上限（ADR-0044），其余显示首帧。 */
private const val PLAYING_LIMIT = 4

/** 静态头像的解码缓存总字节数：头像最大 640×640，十几张够一屏用。 */
private const val STILL_CACHE_BYTES = 16 * 1024 * 1024

/**
 * 头像现在可不可以播（ADR-0044）：App 在前台、这个头像在屏幕上，且没开省电模式和减弱动态效果。
 * 前台看的是本 Activity 的 STARTED；省电和动画缩放是全进程的（[AvatarEnvironment]）。
 */
@Composable
internal fun avatarAllowed(): Boolean {
    AvatarEnvironment.observeSystem(LocalContext.current)
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    var onScreen by remember { mutableStateOf(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) }
    DisposableEffect(lifecycle) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_START) onScreen = true
            if (event == Lifecycle.Event.ON_STOP) onScreen = false
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    return onScreen && !AvatarEnvironment.powerSave && !AvatarEnvironment.reduceMotion
}

/** 播放名额：拿不到就显示首帧（规格第 1 节）。滚出屏幕、进后台时随组合释放。 */
@Composable
internal fun rememberPlayingSlot(allowed: Boolean): Boolean {
    var playing by remember { mutableStateOf(false) }
    DisposableEffect(allowed) {
        val got = allowed && AvatarPlayback.tryAcquire()
        playing = got
        onDispose { if (got) AvatarPlayback.release() }
    }
    return playing
}

/** 全进程的播放名额。拿不到名额的头像显示首帧，不影响别人。 */
private object AvatarPlayback {
    private val slots = Semaphore(PLAYING_LIMIT)

    fun tryAcquire() = slots.tryAcquire()

    fun release() = slots.release()
}

/**
 * 省电模式和减弱动态效果（ADR-0044）：进程里挂一次系统监听，界面上是 Compose 状态。
 * 前台不在这里管——头像自己看 Activity 的 STARTED（[avatarAllowed]）。
 */
internal object AvatarEnvironment {
    /** Android 的“减弱动态效果”：动画时长缩放为 0。 */
    var reduceMotion by mutableStateOf(false)
        private set

    var powerSave by mutableStateOf(false)
        private set

    private var observed = false

    fun observeSystem(context: Context) {
        if (observed) return
        observed = true
        val app = context.applicationContext
        val power = app.getSystemService(Context.POWER_SERVICE) as PowerManager
        app.registerReceiver(
            object : BroadcastReceiver() {
                override fun onReceive(receiverContext: Context, intent: Intent) {
                    powerSave = power.isPowerSaveMode
                }
            },
            IntentFilter(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED),
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
        app.contentResolver.registerContentObserver(
            Settings.Global.getUriFor(Settings.Global.ANIMATOR_DURATION_SCALE),
            false,
            object : android.database.ContentObserver(null) {
                override fun onChange(selfChange: Boolean) {
                    reduceMotion = animatorScaleOff(app)
                }
            },
        )
        powerSave = power.isPowerSaveMode
        reduceMotion = animatorScaleOff(app)
    }

    private fun animatorScaleOff(context: Context) = Settings.Global.getFloat(
        context.contentResolver,
        Settings.Global.ANIMATOR_DURATION_SCALE,
        1f,
    ) == 0f
}

/** 解码缓存按 token 记：核心按 mxc 只下一次，同一张头像不用反复解码（规格第 1 节）。 */
internal object AvatarStillCache {
    private val cache = object : LruCache<String, ImageBitmap>(STILL_CACHE_BYTES) {
        override fun sizeOf(key: String, value: ImageBitmap) = value.asAndroidBitmap().byteCount
    }

    suspend fun load(core: ClientCore, media: MediaHandle): Result<ImageBitmap> =
        cache.get(media.token)?.let { Result.success(it) } ?: withContext(Dispatchers.Default) {
            try {
                val bytes = core.loadMedia(media)
                // ImageDecoder 按 EXIF 方向转正（BitmapFactory 不会）。
                val source = ImageDecoder.createSource(ByteBuffer.wrap(bytes))
                val bitmap = ImageDecoder.decodeBitmap(source)
                val image = bitmap.asImageBitmap()
                cache.put(media.token, image)
                Result.success(image)
            } catch (failure: CoreException) {
                Result.failure(failure)
            } catch (failure: IOException) {
                Result.failure(failure)
            }
        }
}
