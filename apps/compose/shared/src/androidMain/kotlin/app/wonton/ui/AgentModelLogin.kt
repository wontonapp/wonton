package app.wonton.ui

import android.content.ClipData
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.em
import app.wonton.protocol.AgentModelsAction
import app.wonton.protocol.LoginState
import app.wonton.protocol.ModelLoginBrowser
import app.wonton.protocol.ModelLoginProgress
import app.wonton.protocol.ModelLoginState
import app.wonton.protocol.ModelProviderPage
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

private const val CODE_SPACING = 0.12f

/** 登录这一块：没在登录时是按钮（上次没成的原因写在上面），在登录时是码和等待。 */
@Composable
internal fun ModelLoginSection(page: ModelProviderPage, send: (AgentModelsAction) -> Unit, browser: ModelBrowser) {
    val login = page.login
    if (login != null) {
        LoginInProgress(login, send, browser)
        return
    }
    if (!page.deviceLogin && !page.browserLogin) return
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        val outcome = page.loginOutcome
        outcome?.let { LoginOutcomeNotice(it) }
        val again = outcome != null || page.row.connected
        WontonButton(
            stringResource(if (again) R.string.agent_models_login_again else R.string.agent_models_login),
            { send(AgentModelsAction.StartModelLogin) },
            Modifier.fillMaxWidth(),
            kind = if (page.row.connected) WontonButtonKind.SECONDARY else WontonButtonKind.PRIMARY,
            enabled = !page.busy,
        )
    }
}

@Composable
private fun LoginOutcomeNotice(outcome: ModelLoginState) {
    val title = when (outcome.state) {
        LoginState.DENIED -> R.string.agent_models_login_denied
        LoginState.EXPIRED -> R.string.agent_models_login_expired
        else -> R.string.agent_models_login_failed
    }
    WontonNotice(WontonIcon.Warning, stringResource(title)) {
        outcome.message?.takeIf { it.isNotBlank() }?.let { ProblemDetail(it) }
    }
}

@Composable
private fun LoginInProgress(login: ModelLoginProgress, send: (AgentModelsAction) -> Unit, browser: ModelBrowser) {
    val colors = WontonTheme.colors
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    val clipLabel = stringResource(R.string.agent_models_login_code_label)
    Column(
        Modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        val code = login.code
        // 码已经带在网址里：浏览器会替人填好，不再重复显示。
        val showsCode = !code.isNullOrEmpty() && !login.url.contains(code.orEmpty())
        if (showsCode) {
            SelectionContainer {
                Text(
                    code.orEmpty(),
                    style = WontonTheme.typography.screenTitle.copy(
                        fontFamily = FontFamily.Monospace, letterSpacing = CODE_SPACING.em,
                        textAlign = TextAlign.Center,
                    ),
                    color = colors.textPrimary.toColor(),
                )
            }
        }
        if (login.browser == ModelLoginBrowser.OTHER) {
            Text(
                stringResource(R.string.agent_models_login_unsupported),
                style = WontonTheme.typography.supporting, color = colors.danger.toColor(),
            )
        } else {
            LoginWaiting()
            WontonButton(
                stringResource(
                    if (showsCode) R.string.agent_models_login_copy_open else R.string.agent_models_login_open,
                ),
                {
                    if (showsCode) {
                        scope.launch {
                            clipboard.setClipEntry(ClipEntry(ClipData.newPlainText(clipLabel, code.orEmpty())))
                        }
                    }
                    browser.openLogin(login)
                },
                Modifier.fillMaxWidth(),
            )
        }
        WontonButton(
            stringResource(R.string.common_cancel), { send(AgentModelsAction.CancelModelLogin) },
            Modifier.fillMaxWidth(), kind = WontonButtonKind.SECONDARY,
        )
    }
}

@Composable
private fun LoginWaiting() {
    val colors = WontonTheme.colors
    Row(
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CircularProgressIndicator(
            Modifier.size(WontonTheme.size.iconSmall),
            color = colors.textPrimary.toColor(), strokeWidth = WontonTheme.size.hairline,
        )
        Text(
            stringResource(R.string.agent_models_login_waiting),
            style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor(),
        )
    }
}
