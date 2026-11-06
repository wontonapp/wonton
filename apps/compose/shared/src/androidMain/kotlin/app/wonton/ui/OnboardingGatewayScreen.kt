package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource

import app.wonton.protocol.HomeViewModel
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

@Composable
internal fun OnboardingGatewayScreen(url: String, home: HomeViewModel, onClose: () -> Unit) {
    var reload by remember(url) { mutableStateOf(0uL) }
    var loaded by remember(url, reload) { mutableStateOf(false) }
    var failed by remember(url, reload) { mutableStateOf(false) }
    var rejection by remember { mutableStateOf<Problem?>(null) }
    val controller = rememberPluginWebViewController()
    BackHandler { if (!controller.goBack()) onClose() }
    val openLink = externalLinkOpener { rejection = it }
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.onboarding_gateway_title), onClose) }) {
        Box(Modifier.fillMaxSize().navigationBarsPadding()) {
            home.proxy?.let { proxy ->
                PluginWebView(
                    url, proxy, reload,
                    onLoaded = { loaded = true; failed = false },
                    onFailed = { failed = true },
                    controller = controller,
                    onOpenLink = openLink,
                )
            }
            if (!loaded || failed || home.proxy == null) {
                Box(
                    Modifier.fillMaxSize().background(WontonTheme.colors.canvas.toColor())
                        .padding(WontonTheme.spacing.pageInset),
                    contentAlignment = Alignment.Center,
                ) {
                    if (failed || home.proxy == null) {
                        OnboardingProblem(
                            stringResource(
                                if (home.proxy == null) R.string.onboarding_problem_plugin_transport_unavailable
                                else R.string.onboarding_problem_plugin_upstream,
                            ),
                            null,
                        ) { reload++ }
                    } else {
                        OnboardingLoading()
                    }
                }
            }
        }
    }
    rejection?.let { ProblemDialog(it) { rejection = null } }
}
