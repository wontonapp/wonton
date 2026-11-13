package app.wonton.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.MatrixLoginFailure
import app.wonton.protocol.MatrixLoginMethod
import app.wonton.protocol.MatrixLoginState
import app.wonton.protocol.MatrixLoginViewModel
import app.wonton.protocol.MatrixOidcLogin
import app.wonton.protocol.MatrixPasswordLogin
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.icons.Error
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

/**
 * 用 Matrix 账号登录（ADR-0051）。进展只来自 [login]（`ServersViewModel.matrixLogin`）：
 * 壳不猜哪台服务器支持哪种方式。`login` 为空或已结束时回到起手的表单。
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun MatrixLoginScreen(
    login: MatrixLoginViewModel?,
    core: ClientCore,
    server: String,
    onServerChange: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    val active = login?.takeUnless { it.state.isTerminal }
    val cancel = {
        active?.let { core.send(Action.CancelMatrixLogin(it.loginId)) }
        onDismiss()
        Unit
    }
    BackHandler(onBack = cancel)
    WontonScreen(bar = {
        TopAppBar(
            title = { Text(stringResource(R.string.matrix_login_title), style = WontonTheme.typography.itemTitle) },
            navigationIcon = { TextButton(onClick = cancel) { Text(stringResource(R.string.common_cancel)) } },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = WontonTheme.colors.canvas.toColor()),
        )
    }) {
        if (active != null) {
            MatrixLoginFlow(active, core)
        } else {
            MatrixLoginStart(server, onServerChange, core)
        }
    }
}

/** 进行中的一步；重试从新的发现流程开始，所以每一步都能回到顶上的取消。 */
@Composable
private fun MatrixLoginFlow(login: MatrixLoginViewModel, core: ClientCore) {
    // 核心当场拒绝的原因，比如提交被拒。
    var refusal by remember { mutableStateOf<Problem?>(null) }
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        refusal?.let { ProblemNotice(it, WontonIcon.Error) }
        when (val state = login.state) {
            is MatrixLoginState.Discovering ->
                MatrixLoginProgress(stringResource(R.string.matrix_login_contacting, login.homeserver))
            is MatrixLoginState.Ready -> MatrixLoginReady(login, core) { refusal = it }
            is MatrixLoginState.Authenticating ->
                MatrixLoginProgress(stringResource(R.string.matrix_login_signing_in))
            is MatrixLoginState.AwaitingOidc -> MatrixLoginAwaitingOidc(state.authorizationUrl)
            is MatrixLoginState.Saving -> MatrixLoginProgress(stringResource(R.string.matrix_login_saving))
            is MatrixLoginState.Failed ->
                MatrixLoginFailed(state.reason, login.homeserver, core) { refusal = it }
            is MatrixLoginState.Succeeded, is MatrixLoginState.Cancelled -> {}
            is MatrixLoginState.Other ->
                MatrixLoginUnknown(login.homeserver, core) { refusal = it }
        }
    }
}

/** 发现完成：登录哪台服务器是数据不是装饰，列出来；方式按核心给的顺序列。 */
@Composable
private fun MatrixLoginReady(login: MatrixLoginViewModel, core: ClientCore, refuse: (Problem?) -> Unit) {
    val context = LocalContext.current
    val methods = login.methods
    // 只有密码一种方式时表单直接铺开，不隔一个按钮。
    var showPassword by remember(login.loginId) {
        mutableStateOf(MatrixLoginMethod.PASSWORD in methods && MatrixLoginMethod.OIDC !in methods)
    }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap)) {
        Text(
            login.homeserver,
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        if (methods.isEmpty()) {
            WontonNotice(
                WontonIcon.Warning,
                stringResource(R.string.matrix_login_failed_unsupported_method),
            )
        }
        if (MatrixLoginMethod.PASSWORD in methods && !showPassword) {
            WontonButton(
                stringResource(R.string.matrix_login_method_password),
                { showPassword = true },
                Modifier.fillMaxWidth(),
            )
        }
        if (MatrixLoginMethod.OIDC in methods) {
            WontonButton(
                stringResource(R.string.matrix_login_method_oidc),
                {
                    refuse(
                        core.send(
                            Action.StartMatrixOidc(
                                MatrixOidcLogin(
                                    login.loginId,
                                    // 核心接受的回调地址，壳不另写一份；MainActivity 按它认深链。
                                    login.oidcRedirectUri,
                                    defaultDeviceName(context),
                                ),
                            ),
                        ),
                    )
                },
                Modifier.fillMaxWidth(),
                kind = if (MatrixLoginMethod.PASSWORD in methods) {
                    WontonButtonKind.SECONDARY
                } else {
                    WontonButtonKind.PRIMARY
                },
            )
        }
        if (showPassword) {
            MatrixPasswordForm(login, core, refuse)
        }
    }
}

@Composable
private fun MatrixPasswordForm(login: MatrixLoginViewModel, core: ClientCore, refuse: (Problem?) -> Unit) {
    val context = LocalContext.current
    val focus = LocalFocusManager.current
    var userId by rememberSaveable { mutableStateOf("") }
    // 密码不进可恢复的界面状态：旋转后就重新输，也不留在系统备份里。
    var password by remember { mutableStateOf("") }
    val submit = {
        focus.clearFocus()
        refuse(
            core.send(
                Action.SubmitMatrixPassword(
                    MatrixPasswordLogin(login.loginId, userId.trim(), password, defaultDeviceName(context)),
                ),
            ),
        )
        Unit
    }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        WontonTextField(
            userId, { userId = it }, Modifier.fillMaxWidth(),
            label = stringResource(R.string.matrix_login_user_id),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email, imeAction = ImeAction.Next),
        )
        WontonTextField(
            password, { password = it }, Modifier.fillMaxWidth(),
            label = stringResource(R.string.matrix_login_password),
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { submit() }),
        )
        WontonButton(
            stringResource(R.string.matrix_login_sign_in), submit, Modifier.fillMaxWidth(),
            enabled = userId.isNotBlank() && password.isNotEmpty(),
        )
    }
}

/** 登录跳去了系统浏览器；回来靠核心 `oidcRedirectUri` 的深链，按钮补一次手动打开。 */
@Composable
private fun MatrixLoginAwaitingOidc(authorizationUrl: String) {
    val context = LocalContext.current
    var openFailed by remember { mutableStateOf(false) }
    LaunchedEffect(authorizationUrl) { openFailed = !openBrowser(context, authorizationUrl) }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap)) {
        MatrixLoginProgress(stringResource(R.string.matrix_login_finish_in_browser))
        if (openFailed) {
            WontonNotice(WontonIcon.Warning, stringResource(R.string.matrix_login_no_browser))
        }
        WontonButton(
            stringResource(R.string.matrix_login_open_browser),
            { openFailed = !openBrowser(context, authorizationUrl) },
            Modifier.fillMaxWidth(),
            kind = WontonButtonKind.SECONDARY,
        )
    }
}

/** 失败：一句人话加重试；试过哪台服务器和类别收进默认收起的详情。 */
@Composable
private fun MatrixLoginFailed(
    reason: MatrixLoginFailure,
    homeserver: String,
    core: ClientCore,
    refuse: (Problem?) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap)) {
        WontonNotice(WontonIcon.Error, stringResource(failureTitle(reason)))
        ProblemDetail("$homeserver\n${reason.name}")
        WontonButton(
            stringResource(R.string.common_retry),
            {
                refuse(core.send(Action.StartMatrixLogin(homeserver)))
            },
            Modifier.fillMaxWidth(),
        )
    }
}

/** 核心给了壳不认识的状态：如实说不知道，不装成进行中。 */
@Composable
private fun MatrixLoginUnknown(homeserver: String, core: ClientCore, refuse: (Problem?) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap)) {
        WontonNotice(WontonIcon.Warning, stringResource(R.string.matrix_login_unknown_state))
        WontonButton(
            stringResource(R.string.common_retry),
            { refuse(core.send(Action.StartMatrixLogin(homeserver))) },
            Modifier.fillMaxWidth(),
        )
    }
}

@Composable
private fun MatrixLoginProgress(label: String) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        CircularProgressIndicator(
            Modifier.size(WontonTheme.size.iconSmall),
            color = WontonTheme.colors.textSecondary.toColor(),
            strokeWidth = 2.dp,
        )
        Text(label, style = WontonTheme.typography.body, color = WontonTheme.colors.textPrimary.toColor())
    }
}

/** 起手：填服务器（默认 matrix.org），继续后由核心发现支持的登录方式。 */
@Composable
private fun MatrixLoginStart(server: String, onServerChange: (String) -> Unit, core: ClientCore) {
    var refusal by remember { mutableStateOf<Problem?>(null) }
    val focus = LocalFocusManager.current
    val start = {
        focus.clearFocus()
        refusal = core.send(Action.StartMatrixLogin(server.trim().takeIf { it.isNotEmpty() }))
        Unit
    }
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        refusal?.let { ProblemNotice(it, WontonIcon.Error) }
        WontonTextField(
            server, onServerChange, Modifier.fillMaxWidth(),
            label = stringResource(R.string.matrix_login_server),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { start() }),
        )
        WontonButton(stringResource(R.string.matrix_login_continue), start, Modifier.fillMaxWidth())
    }
}

private fun openBrowser(context: Context, url: String): Boolean =
    try {
        context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)))
        true
    } catch (_: ActivityNotFoundException) {
        false
    }
