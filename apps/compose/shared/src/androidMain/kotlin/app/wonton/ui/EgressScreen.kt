package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.CoreException
import app.wonton.protocol.EgressConfirmation
import app.wonton.protocol.EgressCurrent
import app.wonton.protocol.EgressPathState
import app.wonton.protocol.EgressProxySource
import app.wonton.protocol.LanWebuiOffer
import app.wonton.protocol.PluginDetail
import app.wonton.protocol.Rgb
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

/** 出网代理的设置页（ADR-0023）：结论都在核心，这里渲染，并把输入框的内容发回去。 */
@Composable
fun EgressPage(plugins: PluginsState, onBack: () -> Unit) {
    val egress = plugins.viewModel.egress
    val current = egress.current
    LaunchedEffect(Unit) { plugins.send(Action.RefreshEgressProxy) }
    // 核心给的值变了才覆盖输入框：保存失败时主人填的还在。
    var address by remember(current?.address) { mutableStateOf(current?.address.orEmpty()) }
    var directCidrs by remember(current?.directCidrs) { mutableStateOf(current?.directCidrs.orEmpty()) }
    val save = { target: String -> plugins.send(Action.SaveEgressProxy(target, directCidrs)) }
    val bar = @Composable {
        WontonPushedBar("出网代理", onBack) {
            if (egress.saving) {
                CircularProgressIndicator(
                    Modifier.padding(end = WontonTheme.spacing.s12).size(WontonTheme.size.icon),
                    strokeWidth = 2.dp,
                )
            } else {
                Box(Modifier.padding(end = WontonTheme.spacing.s12)) {
                    WontonSmallButton("保存", { save(address) }, enabled = current != null)
                }
            }
        }
    }
    WontonScreen(bar = bar) {
        Box(Modifier.fillMaxSize()) {
            val problem = egress.loadProblem
            when {
                current != null -> Column(
                    Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
                    verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
                ) {
                    listOfNotNull(egress.saveProblem, problem).forEach { WontonNotice(WontonIcon.Warning, it.message) }
                    AddressField(current, address, !egress.saving) { address = it }
                    val plugin = current.pluginPath
                    val core = current.corePath
                    if (plugin != null && core != null) {
                        WontonGroup {
                            EgressPathRow("应用", plugin)
                            WontonSeparator()
                            EgressPathRow("家里的服务器", core)
                        }
                    }
                    RangesField(current, directCidrs, !egress.saving) { directCidrs = it }
                    if (current.settings.url != null && !current.addressLocked) {
                        WontonButton(
                            "取消代理",
                            { save("") },
                            Modifier.fillMaxWidth(),
                            WontonButtonKind.DANGER,
                            enabled = !egress.saving,
                        )
                    }
                }
                problem != null -> ProblemView(problem.problem) { plugins.send(Action.RefreshEgressProxy) }
                else -> CircularProgressIndicator(Modifier.align(Alignment.Center))
            }
        }
    }
    egress.confirmation?.let { RestartDialog(it, plugins) }
}

@Composable
private fun RestartDialog(confirmation: EgressConfirmation, plugins: PluginsState) {
    val dismiss = { plugins.send(Action.DismissEgressConfirmation) }
    WontonDialog(
        if (confirmation.cancelling) "取消代理要重启这些应用" else "配上代理要重启这些应用",
        confirmation.restarts.joinToString("、") { it.title } + "\n数据不动。",
        dismiss,
        listOf(
            WontonDialogAction("取消", onClick = dismiss),
            WontonDialogAction("重启", WontonButtonKind.PRIMARY) { plugins.send(Action.ConfirmEgressProxy) },
        ),
    )
}

@Composable
private fun AddressField(current: EgressCurrent, value: String, enabled: Boolean, onChange: (String) -> Unit) {
    DetailSection("代理地址") {
        WontonTextField(
            value,
            onChange,
            Modifier.fillMaxWidth(),
            placeholder = "http://192.168.1.2:7890",
            label = "代理地址",
            enabled = enabled && !current.addressLocked,
            keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Uri),
        )
        when (current.settings.source) {
            EgressProxySource.ENV -> Secondary("地址由 WONTON_EGRESS_PROXY 指定。要改的话，改安装目录的设置，再运行一次安装脚本。")
            EgressProxySource.DETECTED -> Secondary("安装时检测到的。")
            EgressProxySource.UNSET, EgressProxySource.SETTINGS, EgressProxySource.UNKNOWN -> Unit
        }
    }
}

@Composable
private fun RangesField(current: EgressCurrent, value: String, enabled: Boolean, onChange: (String) -> Unit) {
    DetailSection("直连网段") {
        WontonTextField(
            value,
            onChange,
            Modifier.fillMaxWidth(),
            placeholder = "100.64.0.0/10",
            label = "直连网段",
            enabled = enabled,
            keyboardOptions = KeyboardOptions(autoCorrectEnabled = false),
        )
        if (current.settings.hostCidrs.isNotEmpty()) {
            Secondary("主机所在网段：" + current.settings.hostCidrs.joinToString(", "))
        }
        Secondary("这些网段不经代理，出网应用都能连。")
        current.settings.notices.forEach { Secondary(it, WontonTheme.colors.warning) }
    }
}

/** 一条路径通不通，不通时写原因。 */
@Composable
private fun EgressPathRow(title: String, state: EgressPathState) {
    val colors = WontonTheme.colors
    val (label, tint, message) = when (state) {
        is EgressPathState.Probing -> Triple("探测中", colors.textSecondary, null)
        is EgressPathState.Reachable -> Triple("通", colors.success, null)
        is EgressPathState.Uncertain -> Triple("不确定", colors.warning, state.message)
        is EgressPathState.Failed -> Triple("不通", colors.danger, state.message)
    }
    Column {
        WontonRow(title, label, tint)
        message?.let {
            Box(
                Modifier.padding(
                    start = WontonTheme.spacing.s16,
                    end = WontonTheme.spacing.s16,
                    bottom = WontonTheme.spacing.s12,
                ),
            ) {
                Secondary(it)
            }
        }
    }
}

@Composable
private fun Secondary(text: String, color: Rgb = WontonTheme.colors.textSecondary) {
    Text(text, style = WontonTheme.typography.supporting, color = color.toColor())
}

/** 网页局域网端口的开关（ADR-0023），风险写在旁边。 */
@Composable
fun LanWebuiToggle(offer: LanWebuiOffer, checked: Boolean, enabled: Boolean, onChange: (Boolean) -> Unit) {
    val colors = WontonTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("在局域网开放网页", style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
                Secondary("端口 " + offer.ports.joinToString(", ") { it.port.toString() })
            }
            WontonSwitch(checked, onChange, enabled)
        }
        Secondary(offer.risk, colors.warning)
    }
}

/** 已装插件的开关：核心重建容器期间不能再点，失败时写原因，并重取详情（关失败时插件标为异常）。 */
@Composable
fun InstalledLanWebui(plugins: PluginsState, id: String, offer: LanWebuiOffer, onUpdated: (PluginDetail) -> Unit) {
    val scope = rememberCoroutineScope()
    var target by remember { mutableStateOf<Boolean?>(null) }
    var problem by remember { mutableStateOf<String?>(null) }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        LanWebuiToggle(offer, target ?: offer.enabled, enabled = target == null) { enabled ->
            target = enabled
            problem = null
            scope.launch {
                try {
                    onUpdated(plugins.core.setLanWebui(id, enabled))
                } catch (failure: CoreException) {
                    problem = failure.toString()
                    runCatching { plugins.core.installedPlugin(id) }.onSuccess(onUpdated)
                }
                target = null
            }
        }
        if (target != null) CircularProgressIndicator(Modifier.size(WontonTheme.size.icon), strokeWidth = 2.dp)
        problem?.let { WontonNotice(WontonIcon.Warning, it) }
    }
}
