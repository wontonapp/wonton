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
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.HomeAction
import app.wonton.protocol.Action
import app.wonton.protocol.DeviceInvite
import app.wonton.protocol.KeyHandoff
import app.wonton.protocol.VerificationFlow
import app.wonton.protocol.VerificationStage
import app.wonton.ui.design.wontonContentPadding
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
 * [invite]（`HomeViewModel.invite`）。新设备发起的确认请求（ADR-0046）在配对码出示期间就在这一屏接着显示；
 * 这一页开着时核心替新设备交出聊天密钥（ADR-0087，[handoff]），离开时告诉核心不再交。
 */
@Composable
fun InviteScreen(
    core: ClientCore,
    invite: DeviceInvite?,
    verification: VerificationFlow?,
    handoff: KeyHandoff?,
    modifier: Modifier = Modifier,
) {
    // 核心当场拒绝的原因，比如还没配对。
    var refusal by remember { mutableStateOf<Problem?>(null) }
    val request = { refusal = core.send(Action.Home(HomeAction.InviteDevice)) }
    LaunchedEffect(Unit) { request() }
    DisposableEffect(core) { onDispose { core.send(Action.Home(HomeAction.CloseDeviceInvite)) } }
    val giving = handoff is KeyHandoff.Giving
    Column(
        modifier.verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        verification?.takeIf { !it.startedHere }?.let { flow ->
            if (giving && flow.stage is VerificationStage.Done) {
                Text(
                    stringResource(R.string.invite_device_added),
                    style = WontonTheme.typography.itemTitle,
                    color = WontonTheme.colors.textPrimary.toColor(),
                )
            } else {
                VerificationFlowContent(flow, onAction = { refusal = core.send(it) }, onUseRecoveryKey = null)
            }
        }
        if (handoff is KeyHandoff.CannotGive) {
            WontonNotice(WontonIcon.Warning, stringResource(R.string.invite_cannot_give))
        }
        refusal?.let { ProblemNotice(it, WontonIcon.Error) }
        // 码已经被新设备用掉、这一页在替它交出密钥：不再摆码。
        if (!giving) InviteProgress(invite, request)
    }
}

@Composable
private fun InviteProgress(invite: DeviceInvite?, request: () -> Unit) {
    val colors = WontonTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap)) {
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
                    stringResource(R.string.invite_requesting),
                    style = WontonTheme.typography.body,
                    color = colors.textPrimary.toColor(),
                )
            }
            is DeviceInvite.Ready -> InviteCode(invite.code, invite.expiresAtMs.toLong(), onRenew = request)
            is DeviceInvite.Failed -> WontonNotice(
                WontonIcon.Warning,
                stringResource(R.string.invite_failed_title),
                lines = listOf(invite.message),
            ) {
                WontonButton(stringResource(R.string.invite_try_again), request, kind = WontonButtonKind.SECONDARY)
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
                contentDescription = stringResource(R.string.invite_qr_label),
                modifier = Modifier.size(QR_SIZE).clip(RoundedCornerShape(WontonTheme.radii.control)),
                filterQuality = FilterQuality.None,
            )
            val expiresIn = "%d:%02d".format(remaining / SECONDS_PER_MINUTE, remaining % SECONDS_PER_MINUTE)
            Text(
                stringResource(R.string.invite_expires_in, expiresIn),
                style = WontonTheme.typography.supporting.copy(fontFeatureSettings = "tnum"),
                color = secondary,
            )
        }
    } else {
        WontonNotice(WontonIcon.Warning, stringResource(R.string.invite_expired)) {
            WontonButton(stringResource(R.string.invite_renew), onRenew, kind = WontonButtonKind.SECONDARY)
        }
    }
    Text(
        stringResource(R.string.invite_instructions),
        style = WontonTheme.typography.supporting,
        color = secondary,
    )
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Text(
            stringResource(R.string.invite_paste_fallback),
            style = WontonTheme.typography.supporting,
            color = secondary,
        )
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
internal fun qrBitmap(text: String): ImageBitmap {
    val hints = mapOf(EncodeHintType.MARGIN to 2, EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.M)
    val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 0, 0, hints)
    val pixels = IntArray(matrix.width * matrix.height) { index ->
        if (matrix[index % matrix.width, index / matrix.width]) Color.BLACK else Color.WHITE
    }
    return Bitmap.createBitmap(pixels, matrix.width, matrix.height, Bitmap.Config.ARGB_8888).asImageBitmap()
}
