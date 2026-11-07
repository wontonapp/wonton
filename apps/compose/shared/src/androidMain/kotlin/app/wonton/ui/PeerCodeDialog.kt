package app.wonton.ui

import android.content.Intent
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.wonton.protocol.PeerCode
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

private val QR_SIZE = 260.dp

/** 出示我的码：长期有效，对方扫了或粘贴只是发请求，我在“请求”里同意（ADR-0050）。 */
@Composable
internal fun PeerCodeDialog(code: PeerCode?, onDismiss: () -> Unit, onReset: () -> Unit) {
    WontonDialog(
        stringResource(R.string.peers_show_code),
        onDismiss = onDismiss,
        detail = {
            if (code == null) {
                Column(
                    Modifier.fillMaxWidth().padding(WontonTheme.spacing.s32),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    CircularProgressIndicator(
                        Modifier.size(WontonTheme.size.iconSmall),
                        color = WontonTheme.colors.textPrimary.toColor(),
                        strokeWidth = 2.dp,
                    )
                }
            } else {
                PeerCodeView(code, onReset)
            }
        },
    )
}

@Composable
private fun PeerCodeView(code: PeerCode, onReset: () -> Unit) {
    val context = LocalContext.current
    var resetting by remember { mutableStateOf(false) }
    Column(
        Modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        val qr = remember(code.code) { qrBitmap(code.code) }
        Image(
            bitmap = qr,
            contentDescription = stringResource(R.string.peers_code_qr_label),
            modifier = Modifier.size(QR_SIZE).clip(RoundedCornerShape(WontonTheme.radii.control)),
            filterQuality = FilterQuality.None,
        )
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        ) {
            WontonButton(
                stringResource(R.string.peers_code_reset),
                { resetting = true },
                Modifier.weight(1f),
                WontonButtonKind.DANGER,
            )
            WontonButton(
                stringResource(R.string.peers_code_share),
                // 系统分享面板收码的文字（ADR-0050）；不进剪贴板。
                {
                    val intent = Intent(Intent.ACTION_SEND)
                        .setType("text/plain")
                        .putExtra(Intent.EXTRA_TEXT, code.code)
                    context.startActivity(Intent.createChooser(intent, null))
                },
                Modifier.weight(1f),
            )
        }
    }
    if (resetting) {
        val dismiss = { resetting = false }
        WontonDialog(
            stringResource(R.string.peers_reset_title),
            stringResource(R.string.peers_reset_message),
            dismiss,
            listOf(
                WontonDialogAction(stringResource(R.string.common_cancel), onClick = dismiss),
                WontonDialogAction(stringResource(R.string.peers_code_reset), WontonButtonKind.DANGER) {
                    dismiss()
                    onReset()
                },
            ),
        )
    }
}
