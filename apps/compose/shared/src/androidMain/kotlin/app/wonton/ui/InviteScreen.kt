package app.wonton.ui

import android.graphics.Bitmap
import android.graphics.Color
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.DeviceInvite
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel
import kotlinx.coroutines.delay

/**
 * “添加设备”：向家里的服务器要一个一次性配对码，画成二维码给新设备上的云吞 App 扫。进展和码都来自核心的
 * [invite]（`HomeViewModel.invite`）。
 */
@Composable
fun InviteScreen(core: ClientCore, invite: DeviceInvite?, modifier: Modifier = Modifier) {
    // 核心当场拒绝的原因，比如还没配对。
    var refusal by remember { mutableStateOf<String?>(null) }
    val request = { refusal = core.send(Action.InviteDevice) }
    LaunchedEffect(Unit) { request() }
    Column(
        modifier.verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        refusal?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        when (invite) {
            null, is DeviceInvite.Requesting -> Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                Text("正在向家里的服务器要配对码…")
            }
            is DeviceInvite.Ready -> InviteCode(invite.code, invite.expiresAtMs.toLong(), onRenew = request)
            is DeviceInvite.Failed -> {
                Text(
                    "没要到配对码",
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.titleSmall,
                )
                Text(invite.message)
                TextButton(onClick = request) { Text("再试一次") }
            }
        }
    }
}

/** 码的二维码、文字和剩下的有效期；过期后换一个。 */
@Composable
private fun InviteCode(code: String, expiresAtMs: Long, onRenew: () -> Unit) {
    val now by produceState(System.currentTimeMillis()) {
        while (true) {
            value = System.currentTimeMillis()
            delay(SECOND_MS)
        }
    }
    val remaining = (expiresAtMs - now) / SECOND_MS
    if (remaining > 0) {
        val qr = remember(code) { qrBitmap(code) }
        Image(
            bitmap = qr,
            contentDescription = "配对二维码",
            modifier = Modifier.size(260.dp),
            filterQuality = FilterQuality.None,
        )
        val minutes = remaining / SECONDS_PER_MINUTE
        val seconds = "%02d".format(remaining % SECONDS_PER_MINUTE)
        Text("还有 $minutes:$seconds 过期", style = MaterialTheme.typography.bodySmall)
    } else {
        Text("这个码过期了。")
        TextButton(onClick = onRenew) { Text("换一个码") }
    }
    Text("在新设备上打开云吞 App 扫这个码，它就成为你的设备。码只能用一次。", style = MaterialTheme.typography.bodySmall)
    Text("不能扫码时粘贴这段文字：", style = MaterialTheme.typography.labelMedium)
    SelectionContainer {
        Text(code, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall)
    }
}

private const val SECOND_MS = 1000L
private const val SECONDS_PER_MINUTE = 60

/** 每个模块一个像素，显示时不插值放大，二维码不会糊。 */
private fun qrBitmap(text: String): ImageBitmap {
    val hints = mapOf(EncodeHintType.MARGIN to 2, EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.M)
    val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 0, 0, hints)
    val pixels = IntArray(matrix.width * matrix.height) { index ->
        if (matrix[index % matrix.width, index / matrix.width]) Color.BLACK else Color.WHITE
    }
    return Bitmap.createBitmap(pixels, matrix.width, matrix.height, Bitmap.Config.ARGB_8888).asImageBitmap()
}
