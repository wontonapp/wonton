package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.PeerError
import app.wonton.protocol.PeersState
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.icons.Error
import app.wonton.ui.icons.WontonIcon

/** 扫对方的码：取景框扫，扫不到就粘贴码的文字；扫到后回列表。 */
@Composable
internal fun PeerScanPage(
    core: ClientCore,
    busy: Boolean,
    onBack: () -> Unit,
    onScan: (String) -> Unit,
    onPasteSubmit: (String) -> Unit,
) {
    var pastedCode by rememberSaveable { mutableStateOf("") }
    var submitted by remember { mutableStateOf(false) }
    var refusal by remember { mutableStateOf<Problem?>(null) }
    // 每份状态都收（值相同的也会送到）：码本身不对（输错、扫了自己的）报在输入框下面，改了输入就消失；
    // 别的失败和成功都回列表，照旧按类别显示。
    LaunchedEffect(core) {
        core.peersUpdates().collect { view ->
            val failed = view.state as? PeersState.Failed
            refusal = failed?.takeIf { it.error.isInputError() }?.problem()
            if (submitted) {
                when {
                    failed != null && !failed.error.isInputError() -> onBack()
                    failed == null && view.state == PeersState.Ready -> onBack()
                }
            }
        }
    }
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.peers_scan_code), onBack) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            CameraArea(onCode = onScan)
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                WontonTextField(
                    value = pastedCode,
                    onValueChange = {
                        pastedCode = it
                        refusal = null
                    },
                    modifier = Modifier.fillMaxWidth(),
                    placeholder = stringResource(R.string.peers_paste_placeholder),
                    textStyle = WontonTheme.typography.caption.copy(fontFamily = FontFamily.Monospace),
                    maxLines = PASTED_CODE_LINES,
                    keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Uri),
                )
                WontonButton(
                    stringResource(R.string.peers_paste_submit),
                    onClick = {
                        submitted = true
                        onPasteSubmit(pastedCode)
                    },
                    modifier = Modifier.fillMaxWidth(),
                    enabled = pastedCode.isNotBlank() && !busy,
                )
                refusal?.let { ProblemNotice(it, WontonIcon.Error) }
            }
        }
    }
}

/** 码本身不对、改输入能解决的三类（码被对方重置过也一样，换张新码就好）；连不上、对方拒绝这些照旧按类别显示。 */
private fun PeerError.isInputError() =
    this == PeerError.CODE_INVALID || this == PeerError.CODE_REVOKED || this == PeerError.SELF_PAIRING

/**
 * 重试能改变结果的失败才给“重试”（照 StoreProblem.canRetry 的写法，用在其他服务器页的横幅和整页失败上）：
 * 已经配对的、自己扫自己的、对方拒绝的和权限不够的，重发还是一样；连不上、超限和上游出错可能是暂时的。
 * 码不对的照样给：重试是回到扫码，扫的是另一张码。请求已处理过的没有“重试”，只有一句提醒。
 */
internal fun PeerError.retriesHelpfully() = when (this) {
    PeerError.ALREADY_PAIRED,
    PeerError.SELF_PAIRING,
    PeerError.PEER_REJECTED,
    PeerError.UNAUTHENTICATED,
    -> false
    else -> true
}
