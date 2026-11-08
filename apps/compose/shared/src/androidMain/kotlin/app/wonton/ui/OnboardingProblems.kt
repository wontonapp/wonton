package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import app.wonton.protocol.PluginFailure
import app.wonton.protocol.PluginProblem
import app.wonton.protocol.ModelProviderFailure
import app.wonton.protocol.ModelProviderProblem
import app.wonton.protocol.StoreFailure
import app.wonton.protocol.StoreProblem
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

@Composable
internal fun OnboardingProblem(title: String, detail: String?, retry: (() -> Unit)? = null) {
    WontonNotice(WontonIcon.Warning, title) {
        retry?.let { WontonButton(stringResource(R.string.common_retry), it, kind = WontonButtonKind.SECONDARY) }
        detail?.let { ProblemDetail(it) }
    }
}

@Composable
internal fun OnboardingLoading() {
    val label = stringResource(R.string.onboarding_loading)
    Column(
        Modifier.fillMaxWidth().semantics { contentDescription = label },
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        CircularProgressIndicator(color = WontonTheme.colors.textPrimary.toColor())
    }
}

@Composable
internal fun PluginProblem.onboardingTitle(): String = stringResource(
    when (reason) {
        PluginFailure.TRANSPORT_UNAVAILABLE -> R.string.onboarding_problem_plugin_transport_unavailable
        PluginFailure.UNAUTHORIZED -> R.string.onboarding_problem_plugin_unauthorized
        PluginFailure.SOURCE_UNAVAILABLE -> R.string.onboarding_problem_plugin_source_unavailable
        PluginFailure.NOT_FOUND -> R.string.onboarding_problem_plugin_not_found
        PluginFailure.CHANGED -> R.string.onboarding_problem_plugin_changed
        PluginFailure.UNSUPPORTED -> R.string.onboarding_problem_plugin_unsupported
        PluginFailure.CONFLICT -> R.string.onboarding_problem_plugin_conflict
        PluginFailure.INVALID -> R.string.onboarding_problem_plugin_invalid
        PluginFailure.DEVICE_UNAVAILABLE -> R.string.onboarding_problem_plugin_device_unavailable
        PluginFailure.LAN_UNAVAILABLE -> R.string.onboarding_problem_plugin_lan_unavailable
        PluginFailure.LAN_PORT_TAKEN -> R.string.onboarding_problem_plugin_lan_port_taken
        PluginFailure.CORE_UPGRADING -> R.string.onboarding_problem_plugin_core_upgrading
        PluginFailure.EGRESS_PROXY_UNAVAILABLE -> R.string.onboarding_problem_plugin_egress_proxy_unavailable
        PluginFailure.OWNER_PROFILE_INCOMPLETE -> R.string.onboarding_problem_plugin_owner_profile_incomplete
        PluginFailure.UPSTREAM -> R.string.onboarding_problem_plugin_upstream
        PluginFailure.PROTOCOL -> R.string.onboarding_problem_plugin_protocol
        // 比这个壳新的类别：先按服务器出错说，壳以后再补一条。
        else -> R.string.onboarding_problem_plugin_upstream
    },
)

@Composable
internal fun StoreProblem.onboardingTitle(): String = stringResource(
    when (reason) {
        StoreFailure.UNREACHABLE -> R.string.onboarding_problem_store_unreachable
        StoreFailure.TIMEOUT -> R.string.onboarding_problem_store_timeout
        StoreFailure.RATE_LIMITED -> R.string.onboarding_problem_store_rate_limited
        StoreFailure.INVALID_INDEX -> R.string.onboarding_problem_store_invalid_index
        StoreFailure.INDEX_TOO_NEW -> R.string.onboarding_problem_store_index_too_new
        StoreFailure.UPSTREAM -> R.string.onboarding_problem_store_upstream
        StoreFailure.INCOMPLETE -> R.string.onboarding_problem_store_incomplete
        StoreFailure.DISABLED -> R.string.onboarding_problem_store_disabled
        StoreFailure.OTHER -> R.string.onboarding_problem_store_other
    },
)

@Composable
internal fun ModelProviderProblem.onboardingTitle(): String = stringResource(
    when (reason) {
        ModelProviderFailure.KEY_REJECTED -> R.string.onboarding_problem_model_key_rejected
        ModelProviderFailure.PROVIDER_UNREACHABLE -> R.string.onboarding_problem_model_provider_unreachable
        ModelProviderFailure.GATEWAY_NOT_READY -> R.string.onboarding_problem_model_gateway_not_ready
        ModelProviderFailure.TRANSPORT_UNAVAILABLE -> R.string.onboarding_problem_model_transport_unavailable
        ModelProviderFailure.UNAUTHORIZED -> R.string.onboarding_problem_model_unauthorized
        ModelProviderFailure.UPSTREAM -> R.string.onboarding_problem_model_upstream
        ModelProviderFailure.PROTOCOL -> R.string.onboarding_problem_model_protocol
    },
)
