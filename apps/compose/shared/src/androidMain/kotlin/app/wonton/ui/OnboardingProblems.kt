package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
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
        retry?.let { WontonButton("重试", it, kind = WontonButtonKind.SECONDARY) }
        detail?.let { ProblemDetail(it) }
    }
}

@Composable
internal fun OnboardingLoading() {
    val label = "正在加载"
    Column(
        Modifier.fillMaxWidth().semantics { contentDescription = label },
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        CircularProgressIndicator(color = WontonTheme.colors.textPrimary.toColor())
    }
}

internal fun PluginProblem.onboardingTitle(): String = when (reason) {
    PluginFailure.TRANSPORT_UNAVAILABLE -> "连不上家里的服务器，检查网络后重试。"
    PluginFailure.UNAUTHORIZED -> "家里的服务器不认这台设备，请重新扫码配对。"
    PluginFailure.SOURCE_UNAVAILABLE -> "取不到应用的信息，来源恢复后再试。"
    PluginFailure.NOT_FOUND -> "没有找到这个应用，刷新列表后再试。"
    PluginFailure.CHANGED -> "应用在确认之后变了，请重新查看、确认。"
    PluginFailure.UNSUPPORTED -> "家里的服务器不支持这个应用，请先升级服务器。"
    PluginFailure.CONFLICT -> "这个应用已经装了或正在忙，等当前任务结束后刷新。"
    PluginFailure.INVALID -> "这个操作没被接受，查看详情后修改重试。"
    PluginFailure.DEVICE_UNAVAILABLE -> "选择的设备不可用，请重新选择。"
    PluginFailure.LAN_UNAVAILABLE -> "没有可用的局域网地址，请检查服务器的网络设置。"
    PluginFailure.CORE_UPGRADING -> "家里的服务器正在升级，结束后再试。"
    PluginFailure.EGRESS_PROXY_UNAVAILABLE -> "服务器的代理连不上，请检查后重试。"
    PluginFailure.OWNER_PROFILE_INCOMPLETE -> "你的资料还没填好，填好后再试。"
    PluginFailure.UPSTREAM -> "家里的服务器出错了，稍后再试。"
    PluginFailure.PROTOCOL -> "App 和家里的服务器版本对不上，请更新 App 或服务器。"
}

internal fun StoreProblem.onboardingTitle(): String = when (reason) {
    StoreFailure.UNREACHABLE -> "连不上推荐清单的来源，可以重试或跳过。"
    StoreFailure.TIMEOUT -> "推荐清单的来源响应太慢，可以重试或跳过。"
    StoreFailure.RATE_LIMITED -> "推荐清单的请求太频繁，可以稍后重试或跳过。"
    StoreFailure.INVALID_INDEX -> "读不懂推荐清单，可以重试或跳过。"
    StoreFailure.INDEX_TOO_NEW -> "推荐清单需要更新的服务器，可以升级或跳过。"
    StoreFailure.UPSTREAM -> "推荐清单的来源出错了，可以重试或跳过。"
    StoreFailure.INCOMPLETE -> "部分推荐清单没有加载出来，可以重试或跳过。"
    StoreFailure.DISABLED -> "推荐清单的来源已关闭，可以跳过。"
    StoreFailure.OTHER -> "取不到推荐清单，可以重试或跳过。"
}

internal fun ModelProviderProblem.onboardingTitle(): String = when (reason) {
    ModelProviderFailure.KEY_REJECTED -> "服务商拒绝了这个 key，请修改后重新测试。"
    ModelProviderFailure.PROVIDER_UNREACHABLE -> "连不上服务商，请检查地址或网络后重试。"
    ModelProviderFailure.GATEWAY_NOT_READY -> "模型网关还没就绪，请重试或跳过这一步。"
    ModelProviderFailure.TRANSPORT_UNAVAILABLE -> "连不上家里的服务器，请检查网络后重试。"
    ModelProviderFailure.UNAUTHORIZED -> "家里的服务器没有授权这台设备，请检查访问权限后重试。"
    ModelProviderFailure.UPSTREAM -> "家里的服务器没能完成请求，请稍后重试。"
    ModelProviderFailure.PROTOCOL -> "App 和家里服务器的版本不兼容，请更新后重试。"
}
