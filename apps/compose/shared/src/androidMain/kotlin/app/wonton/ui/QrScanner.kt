package app.wonton.ui

import android.util.Log
import android.content.pm.ApplicationInfo
import androidx.activity.compose.LocalActivity
import androidx.camera.core.CameraInfoUnavailableException
import androidx.camera.core.CameraSelector
import androidx.camera.core.CameraState
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.zxing.BinaryBitmap
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import java.util.concurrent.ExecutionException
import java.util.concurrent.Executors

/**
 * 后置相机的取景画面：在每一帧里找二维码，把码里的文字交给 [onCode]；同一个码只报一次，扫到别的码以后才会再报。
 * [singleScan] stops analysis before delivering the first code; other scanners retain continuous scanning.
 * 相机打不开或中途出错时调用 [onUnavailable]，参数是给人看的原因。相机权限由调用方先取得。
 */
@Composable
fun QrScanner(
    onCode: (String) -> Unit,
    onUnavailable: (String) -> Unit,
    modifier: Modifier = Modifier,
    singleScan: Boolean = false,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val activity = LocalActivity.current
    val currentOnCode by rememberUpdatedState(onCode)
    val currentOnUnavailable by rememberUpdatedState(onUnavailable)
    // 默认的 SurfaceView 不受 Compose 的裁剪，放大填满后会盖住取景框上下的内容（Android 模拟器上实测）；TextureView 照常裁剪。
    val previewView = remember(context) {
        PreviewView(context).apply { implementationMode = PreviewView.ImplementationMode.COMPATIBLE }
    }
    DisposableEffect(previewView, lifecycleOwner, singleScan) {
        val camera = QrCamera(previewView, { currentOnCode(it) }, { currentOnUnavailable(it) }, singleScan)
        // Debug-only injection enters the same stop-before-delivery path as the camera decoder.
        val debuggable = context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        val injected = if (singleScan && debuggable) {
            activity?.intent?.getStringExtra("wonton_scan_code")
        } else null
        if (injected != null) {
            activity?.intent?.removeExtra("wonton_scan_code")
            camera.deliver(injected)
        } else {
            camera.start(lifecycleOwner)
        }
        onDispose { camera.stop() }
    }
    AndroidView(factory = { previewView }, modifier = modifier.clipToBounds())
}

private const val TAG = "QrScanner"

/** 一次取景，跟着界面的生命周期开关相机。回调都在主线程上。 */
private class QrCamera(
    private val previewView: PreviewView,
    private val onCode: (String) -> Unit,
    private val onUnavailable: (String) -> Unit,
    private val singleScan: Boolean,
) {
    private fun reason(res: Int): String = previewView.context.getString(res)
    private val main = ContextCompat.getMainExecutor(previewView.context)
    private val frames = Executors.newSingleThreadExecutor()
    private val preview = Preview.Builder().build()
    private val analysis = ImageAnalysis.Builder()
        .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
        .build()
    private var provider: ProcessCameraProvider? = null
    private var stopped = false

    fun start(owner: LifecycleOwner) {
        val future = ProcessCameraProvider.getInstance(previewView.context)
        future.addListener({
            // 相机服务准备好之前界面可能已经离开了。
            if (stopped) return@addListener
            try {
                bind(future.get(), owner)
            } catch (failure: ExecutionException) {
                onCameraFailed(failure)
            }
        }, main)
    }

    fun stop() {
        stopped = true
        provider?.unbind(preview, analysis)
        analysis.clearAnalyzer()
        frames.shutdown()
    }

    fun deliver(text: String) {
        if (stopped) return
        if (singleScan) stop()
        onCode(text)
    }

    private fun bind(provider: ProcessCameraProvider, owner: LifecycleOwner) {
        this.provider = provider
        val problem = try {
            if (provider.hasCamera(CameraSelector.DEFAULT_BACK_CAMERA)) null else R.string.qr_scanner_no_back_camera
        } catch (failure: CameraInfoUnavailableException) {
            Log.w(TAG, "Failed to read camera info", failure)
            R.string.qr_scanner_camera_failed
        }
        if (problem != null) {
            onUnavailable(reason(problem))
            return
        }
        preview.surfaceProvider = previewView.surfaceProvider
        analysis.setAnalyzer(frames, QrAnalyzer(singleScan) { text -> main.execute { deliver(text) } })
        try {
            val camera = provider.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
            camera.cameraInfo.cameraState.observe(owner) { state ->
                state.error?.let { onUnavailable(describe(it)) }
            }
        } catch (failure: IllegalStateException) {
            onCameraFailed(failure)
        }
    }

    /** 异常的原文只有开发者看得懂，留在日志里。 */
    private fun onCameraFailed(failure: Exception) {
        Log.w(TAG, "Failed to open camera", failure)
        onUnavailable(reason(R.string.qr_scanner_camera_failed))
    }

    private fun describe(error: CameraState.StateError): String = when (error.code) {
        CameraState.ERROR_CAMERA_IN_USE, CameraState.ERROR_MAX_CAMERAS_IN_USE ->
            reason(R.string.qr_scanner_camera_in_use)
        CameraState.ERROR_CAMERA_DISABLED -> reason(R.string.qr_scanner_camera_disabled)
        CameraState.ERROR_DO_NOT_DISTURB_MODE_ENABLED -> reason(R.string.qr_scanner_do_not_disturb)
        else -> previewView.context.getString(R.string.qr_scanner_camera_error_code, error.code)
    }
}

/** 在每一帧的亮度平面里找二维码；同一个码只报一次。 */
private class QrAnalyzer(singleScan: Boolean, private val onCode: (String) -> Unit) : ImageAnalysis.Analyzer {
    private val reader = QRCodeReader()
    private val gate = QrScanGate(singleScan)

    override fun analyze(image: ImageProxy) {
        if (gate.finished) {
            image.close()
            return
        }
        val text = image.use(::decode) ?: return
        if (gate.accept(text)) onCode(text)
    }

    private fun decode(frame: ImageProxy): String? {
        val plane = frame.planes[0]
        val bytes = ByteArray(plane.buffer.remaining()).also { plane.buffer.get(it) }
        // 每行可能有填充，按行跨度取亮度，裁成画面的宽度。
        val source = PlanarYUVLuminanceSource(
            bytes, plane.rowStride, frame.height, 0, 0, frame.width, frame.height, false,
        )
        return try {
            reader.decode(BinaryBitmap(HybridBinarizer(source))).text
        } catch (ignored: ReaderException) {
            null
        } finally {
            reader.reset()
        }
    }
}
