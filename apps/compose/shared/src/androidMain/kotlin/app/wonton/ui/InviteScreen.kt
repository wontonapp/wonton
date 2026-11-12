package app.wonton.ui

import android.graphics.Bitmap
import android.graphics.Color
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.DeviceInvite
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Error
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
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
    var refusal by remember { mutableStateOf<Problem?>(null) }
    val request = { refusal = core.send(Action.InviteDevice) }
    LaunchedEffect(Unit) { request() }
    val colors = WontonTheme.colors
    Column(
        modifier.verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        refusal?.let { ProblemNotice(it, WontonIcon.Error) }
        when (invite) {
            null, is DeviceInvite.Requesting -> Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            ) {
                CircularProgressIndicator(
                    Modifier.size(WontonTheme.size.iconSmall),
                    color = colors.textPrimary.toColor(),
                    strokeWidth = 2.dp,
                )
                Text(
                    "正在向家里的服务器要配对码…",
                    style = WontonTheme.typography.body,
                    color = colors.textPrimary.toColor(),
                )
            }
            is DeviceInvite.Ready -> InviteCode(invite.code, invite.expiresAtMs.toLong(), onRenew = request)
            is DeviceInvite.Failed -> WontonNotice(WontonIcon.Warning, "没要到配对码", lines = listOf(invite.message)) {
                WontonButton("再试一次", request, kind = WontonButtonKind.SECONDARY)
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
    val colors = WontonTheme.colors
    val secondary = colors.textSecondary.toColor()
    val remaining = (expiresAtMs - now) / SECOND_MS
    if (remaining > 0) {
        Column(
            Modifier.fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            val qr = remember(code) { qrBitmap(code) }
            Image(
                bitmap = qr,
                contentDescription = "配对二维码",
                modifier = Modifier.size(QR_SIZE).clip(RoundedCornerShape(WontonTheme.radii.control)),
                filterQuality = FilterQuality.None,
            )
            val minutes = remaining / SECONDS_PER_MINUTE
            val seconds = "%02d".format(remaining % SECONDS_PER_MINUTE)
            Text(
                "还有 $minutes:$seconds 过期",
                style = WontonTheme.typography.supporting.copy(fontFeatureSettings = "tnum"),
                color = secondary,
            )
        }
    } else {
        WontonNotice(WontonIcon.Warning, "这个码过期了。") {
            WontonButton("换一个码", onRenew, kind = WontonButtonKind.SECONDARY)
        }
    }
    Text(
        "在新设备上打开云吞 App 扫这个码，它就成为你的设备。码只能用一次。",
        style = WontonTheme.typography.supporting,
        color = secondary,
    )
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Text("不能扫码时粘贴这段文字", style = WontonTheme.typography.supporting, color = secondary)
        SelectionContainer {
            Text(
                code,
                Modifier.fillMaxWidth()
                    .background(colors.surfaceSubtle.toColor(), RoundedCornerShape(WontonTheme.radii.control))
                    .padding(WontonTheme.spacing.s12),
                style = WontonTheme.typography.caption.copy(fontFamily = FontFamily.Monospace),
                color = colors.textPrimary.toColor(),
            )
        }
    }
}

private val QR_SIZE = 260.dp
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
