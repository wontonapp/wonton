// 添加设备页（#666）的收起、展开两态：只进测试源集，不连核心、不加载 .so。
package app.wonton.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.tooling.preview.Preview
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

// 长度和真码相当（`wonton://pair?…`，example.org 和 192.0.2.10 的假地址），看得出展开后的断行。
private const val PREVIEW_CODE =
    "wonton://pair?v=0&id=9f2ce1a4b7d04e5f8a3c6d1b0e9f27a4&relays=https%3A%2F%2Frelay.example.org%3A443" +
        "&addr=192.0.2.10%3A3478&domain=wonton.example.org&core=https%3A%2F%2Fwonton.example.org%2F" +
        "&secret=u7Xk2pQ9wR4tY6uI8oP1aS3dF5gH7jK9lZ2xC4vB6n8"  // gitleaks:allow：写死的假码，不是凭据

@Preview(name = "添加设备（收起）", locale = "en", widthDp = 390, heightDp = 844)
@Composable
internal fun InviteCodeCollapsedPreview() {
    PreviewFrame { InviteCodeBody(fullText = false) }
}

@Preview(name = "添加设备（展开）", locale = "en", widthDp = 390, heightDp = 844)
@Composable
internal fun InviteCodeExpandedPreview() {
    PreviewFrame { InviteCodeBody(fullText = true) }
}

/** 说明和收起行，是页面的下半段；二维码和倒计时不在这两态的渲染范围里。 */
@Composable
private fun InviteCodeBody(fullText: Boolean) {
    Column(Modifier.fillMaxWidth().padding(WontonTheme.spacing.pageInset)) {
        Text(
            stringResource(R.string.invite_instructions),
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        InviteCodeDetails(PREVIEW_CODE, fullText, onToggle = {}, onCopy = {})
    }
}
