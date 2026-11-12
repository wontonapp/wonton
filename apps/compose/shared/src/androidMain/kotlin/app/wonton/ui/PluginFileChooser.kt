package app.wonton.ui

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.provider.MediaStore
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebView
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect

/** 网页要开文件选择：旧请求先答空（每个回调必须答一次，不答页面就一直等）；返回 false 表示这页不处理。 */
internal fun PluginWebViewController.showFileChooser(
    owner: WebView,
    callback: ValueCallback<Array<Uri>>,
    params: WebChromeClient.FileChooserParams,
): Boolean {
    if (view !== owner) return false
    val launcher = fileChooserLauncher
    val intent = fileChooserIntent(params)?.takeIf { launcher != null }
    pendingFileChooser?.onReceiveValue(null)
    if (intent == null) {
        pendingFileChooser = null
        callback.onReceiveValue(null)
    } else {
        pendingFileChooser = callback
        launcher?.launch(intent)
    }
    return true
}

internal fun PluginWebViewController.onFileChooserResult(resultCode: Int, data: Intent?) {
    val callback = pendingFileChooser ?: return
    pendingFileChooser = null
    callback.onReceiveValue(fileChooserResult(resultCode, data))
}

internal fun PluginWebViewController.clearFileChooserLauncher(launcher: ActivityResultLauncher<Intent>?) {
    if (fileChooserLauncher === launcher) fileChooserLauncher = null
}

/** 组合里注册系统选择的结果回调；页面销毁后不再持有。 */
@Composable
internal fun rememberFileChooserLauncher(controller: PluginWebViewController) {
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        controller.onFileChooserResult(result.resultCode, result.data)
    }
    DisposableEffect(controller) {
        controller.fileChooserLauncher = launcher
        onDispose { controller.clearFileChooserLauncher(launcher) }
    }
}

/** capture 是页面点名要相机/录音；其余走系统选择器（accept 类型、多选由 createIntent 带）。 */
private fun PluginWebViewController.fileChooserIntent(params: WebChromeClient.FileChooserParams): Intent? {
    val accept = params.acceptTypes?.firstOrNull { it.isNotBlank() }
    if (params.isCaptureEnabled) {
        return when {
            accept?.startsWith("video/") == true -> Intent(MediaStore.ACTION_VIDEO_CAPTURE)
            accept?.startsWith("audio/") == true -> Intent(MediaStore.Audio.Media.RECORD_SOUND_ACTION)
            else -> Intent(MediaStore.ACTION_IMAGE_CAPTURE)
        }
    }
    return try {
        params.createIntent()
    } catch (_: ActivityNotFoundException) {
        null
    }
}

/** 系统多选放 clipData，单选和拍照在 data.data；parseResult 只认单选，多选要自己拆。 */
private fun PluginWebViewController.fileChooserResult(resultCode: Int, data: Intent?): Array<Uri>? {
    val clip = data?.clipData
    return when {
        resultCode != Activity.RESULT_OK || data == null -> null
        clip != null && clip.itemCount > 0 -> Array(clip.itemCount) { clip.getItemAt(it).uri }
        else -> WebChromeClient.FileChooserParams.parseResult(resultCode, data)
    }
}
