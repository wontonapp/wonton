package app.wonton.ui

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.Settings
import android.webkit.PermissionRequest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.core.content.ContextCompat
import app.wonton.client.TabMediaPermission
import app.wonton.protocol.CoreException
import app.wonton.protocol.PlacedTab
import app.wonton.protocol.TabMediaPrompt
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction

/** 网页要摄像头/麦克风的处置：先问核心（按插件记决定），按回答放行或拒绝；放行还要系统权限。 */
internal class TabMediaPermissionFlow(private val tabs: TabsState, private val place: PlacedTab) {
    fun handle(request: PermissionRequest) {
        val resources = request.resources
        // 只处理摄像头和麦克风；其他资源照旧拒绝，混着的整单拒。
        if (resources.isEmpty() || resources.any {
                it != PermissionRequest.RESOURCE_VIDEO_CAPTURE &&
                    it != PermissionRequest.RESOURCE_AUDIO_CAPTURE
            }
        ) {
            request.deny()
            return
        }
        val camera = PermissionRequest.RESOURCE_VIDEO_CAPTURE in resources
        val microphone = PermissionRequest.RESOURCE_AUDIO_CAPTURE in resources
        // 源从 WebView 的原生回调取，核心按页面自己的源核对；页面里嵌的其他源核心直接拒。
        val decision = request.origin?.toString()?.let { origin ->
            try {
                tabs.core.tabMediaPermissionRequest(place, origin, camera, microphone)
            } catch (_: CoreException) {
                null
            }
        }
        when (decision) {
            TabMediaPermission.ALLOW -> if (tabs.mediaSystemRequest == null) {
                tabs.mediaSystemRequest = request
            } else {
                request.deny()
            }
            TabMediaPermission.PROMPT -> {
                // 同一页换了一份请求：旧的那份没人再等，拒掉。
                tabs.mediaWebRequest?.deny()
                tabs.mediaWebRequest = request
            }
            else -> request.deny()
        }
    }
}

/** 摄像头/麦克风的询问弹窗、系统运行时权限和系统层拒绝后的提示。整个 App 挂一份。 */
@Composable
internal fun TabMediaPermissions(tabs: TabsState) {
    val context = LocalContext.current
    var systemDenied by remember { mutableStateOf(false) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { grants ->
        val request = tabs.mediaSystemRequest
        tabs.mediaSystemRequest = null
        if (request == null) return@rememberLauncherForActivityResult
        if (grants.values.all { it }) request.grant(request.resources) else {
            request.deny()
            systemDenied = true
        }
    }
    // 核心放行、系统权限还没给：向系统申请。系统层拒过的不再弹，直接回拒绝，由下面的提示接住。
    LaunchedEffect(tabs.mediaSystemRequest) {
        val request = tabs.mediaSystemRequest ?: return@LaunchedEffect
        val needed = missingSystemPermissions(context, request)
        if (needed.isEmpty()) answerSystemRequest(tabs) else launcher.launch(needed.toTypedArray())
    }
    // 核心把询问清了（页面关了）：没人可答的网页请求拒掉，别让网页干等。
    LaunchedEffect(tabs.viewModel.mediaPrompt) {
        if (tabs.viewModel.mediaPrompt == null) {
            tabs.mediaWebRequest?.deny()
            tabs.mediaWebRequest = null
        }
    }
    tabs.viewModel.mediaPrompt?.let { prompt -> TabMediaPromptDialog(tabs, prompt) }
    if (systemDenied) {
        WontonDialog(
            stringResource(R.string.tab_media_system_denied_title),
            stringResource(R.string.tab_media_system_denied),
            onDismiss = { systemDenied = false },
            actions = listOf(
                WontonDialogAction(stringResource(R.string.tab_media_system_settings), onClick = {
                    systemDenied = false
                    context.startActivity(appSettingsIntent(context))
                }),
                WontonDialogAction(stringResource(R.string.common_close), onClick = { systemDenied = false }),
            ),
        )
    }
}

@Composable
private fun TabMediaPromptDialog(tabs: TabsState, prompt: TabMediaPrompt) {
    val answer: (Boolean) -> Unit = { allow ->
        val decision = try {
            tabs.core.tabMediaPermissionDecide(allow)
        } catch (_: CoreException) {
            null
        }
        val request = tabs.mediaWebRequest
        tabs.mediaWebRequest = null
        when {
            decision == TabMediaPermission.ALLOW && request != null ->
                tabs.mediaSystemRequest = request
            request != null -> request.deny()
        }
    }
    WontonDialog(
        title = stringResource(prompt.titleRes(), prompt.pluginTitle),
        onDismiss = { answer(false) },
        actions = listOf(
            WontonDialogAction(stringResource(R.string.tab_media_deny), onClick = { answer(false) }),
            WontonDialogAction(
                stringResource(R.string.tab_media_allow),
                kind = WontonButtonKind.PRIMARY,
                onClick = { answer(true) },
            ),
        ),
    )
}

private fun TabMediaPrompt.titleRes(): Int = when {
    camera && microphone -> R.string.tab_media_prompt_both
    camera -> R.string.tab_media_prompt_camera
    else -> R.string.tab_media_prompt_microphone
}

private fun answerSystemRequest(tabs: TabsState) {
    val request = tabs.mediaSystemRequest
    tabs.mediaSystemRequest = null
    request?.grant(request.resources)
}

internal fun missingSystemPermissions(context: Context, request: PermissionRequest): List<String> = buildList {
    if (PermissionRequest.RESOURCE_VIDEO_CAPTURE in request.resources &&
        ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED
    ) {
        add(Manifest.permission.CAMERA)
    }
    if (PermissionRequest.RESOURCE_AUDIO_CAPTURE in request.resources &&
        ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) !=
            PackageManager.PERMISSION_GRANTED
    ) {
        add(Manifest.permission.RECORD_AUDIO)
    }
}

internal fun appSettingsIntent(context: Context) =
    Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
