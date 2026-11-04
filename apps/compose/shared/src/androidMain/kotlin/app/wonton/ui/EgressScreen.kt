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
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
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
import kotlinx.coroutines.launch

/** 出网代理的设置页（ADR-0023）：结论都在核心，这里渲染，并把输入框的内容发回去。 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun EgressPage(plugins: PluginsState, onBack: () -> Unit) {
    val egress = plugins.viewModel.egress
    val current = egress.current
    LaunchedEffect(Unit) { plugins.send(Action.RefreshEgressProxy) }
    // 核心给的值变了才覆盖输入框：保存失败时主人填的还在。
    var address by remember(current?.address) { mutableStateOf(current?.address.orEmpty()) }
    var directCidrs by remember(current?.directCidrs) { mutableStateOf(current?.directCidrs.orEmpty()) }
    val save = { target: String -> plugins.send(Action.SaveEgressProxy(target, directCidrs)) }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("出网代理") },
                navigationIcon = { TextButton(onClick = onBack) { Text("返回") } },
                actions = {
                    if (egress.saving) {
                        CircularProgressIndicator(Modifier.padding(end = 16.dp).size(24.dp), strokeWidth = 2.dp)
                    } else {
                        TextButton(onClick = { save(address) }, enabled = current != null) { Text("保存") }
                    }
                },
            )
        },
    ) { padding ->
        Box(Modifier.padding(padding).fillMaxSize()) {
            val problem = egress.loadProblem
            when {
                current != null -> Column(
                    Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(20.dp),
                ) {
                    listOfNotNull(egress.saveProblem, problem).forEach {
                        Text(it.message, color = WontonTheme.colors.danger.toColor())
                    }
                    AddressField(current, address, !egress.saving) { address = it }
                    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        current.pluginPath?.let { EgressPathRow("插件", it) }
                        current.corePath?.let { EgressPathRow("家里的服务器", it) }
                    }
                    RangesField(current, directCidrs, !egress.saving) { directCidrs = it }
                    if (current.settings.url != null && !current.addressLocked) {
                        TextButton(onClick = { save("") }, enabled = !egress.saving) {
                            Text("取消代理", color = WontonTheme.colors.danger.toColor())
                        }
                    }
                }
                problem != null -> ProblemView(problem.message, null) { plugins.send(Action.RefreshEgressProxy) }
                else -> CircularProgressIndicator(Modifier.align(Alignment.Center))
            }
        }
    }
    egress.confirmation?.let { RestartDialog(it, plugins) }
}

@Composable
private fun RestartDialog(confirmation: EgressConfirmation, plugins: PluginsState) {
    AlertDialog(
        onDismissRequest = { plugins.send(Action.DismissEgressConfirmation) },
        title = { Text(if (confirmation.cancelling) "取消代理要重启这些插件" else "配上代理要重启这些插件") },
        text = { Text(confirmation.restarts.joinToString("、") { it.title } + "\n数据不动。") },
        confirmButton = { TextButton(onClick = { plugins.send(Action.ConfirmEgressProxy) }) { Text("重启") } },
        dismissButton = { TextButton(onClick = { plugins.send(Action.DismissEgressConfirmation) }) { Text("取消") } },
    )
}

@Composable
private fun AddressField(current: EgressCurrent, value: String, enabled: Boolean, onChange: (String) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        OutlinedTextField(
            value = value,
            onValueChange = onChange,
            modifier = Modifier.fillMaxWidth(),
            enabled = enabled && !current.addressLocked,
            label = { Text("代理地址") },
            placeholder = { Text("http://192.168.1.2:7890") },
            singleLine = true,
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
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        OutlinedTextField(
            value = value,
            onValueChange = onChange,
            modifier = Modifier.fillMaxWidth(),
            enabled = enabled,
            label = { Text("直连网段") },
            placeholder = { Text("100.64.0.0/10") },
            singleLine = true,
            keyboardOptions = KeyboardOptions(autoCorrectEnabled = false),
        )
        if (current.settings.hostCidrs.isNotEmpty()) {
            Secondary("主机所在网段：" + current.settings.hostCidrs.joinToString(", "))
        }
        Secondary("这些网段不经代理，出网插件都能连。")
        current.settings.notices.forEach { Text(it, color = WontonTheme.colors.warning.toColor()) }
    }
}

/** 一条路径通不通，不通时写原因。 */
@Composable
private fun EgressPathRow(title: String, state: EgressPathState) {
    val colors = WontonTheme.colors
    val (label, tint, message) = when (state) {
        is EgressPathState.Probing -> Triple("探测中", colors.textSecondary.toColor(), null)
        is EgressPathState.Reachable -> Triple("通", colors.success.toColor(), null)
        is EgressPathState.Uncertain -> Triple("不确定", colors.warning.toColor(), state.message)
        is EgressPathState.Failed -> Triple("不通", colors.danger.toColor(), state.message)
    }
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(title)
            Text(label, color = tint, fontWeight = FontWeight.SemiBold)
        }
        message?.let { Secondary(it) }
    }
}

@Composable
private fun Secondary(text: String, color: Color = WontonTheme.colors.textSecondary.toColor()) {
    Text(text, style = MaterialTheme.typography.bodySmall, color = color)
}

/** 网页局域网端口的开关（ADR-0023），风险写在旁边。 */
@Composable
fun LanWebuiToggle(offer: LanWebuiOffer, checked: Boolean, enabled: Boolean, onChange: (Boolean) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("在局域网开放网页")
                Secondary("端口 " + offer.ports.joinToString(", ") { it.port.toString() })
            }
            Switch(checked, onChange, enabled = enabled)
        }
        Secondary(offer.risk, WontonTheme.colors.warning.toColor())
    }
}

/** 已装插件的开关：核心重建容器期间不能再点，失败时写原因，并重取详情（关失败时插件标为异常）。 */
@Composable
fun InstalledLanWebui(plugins: PluginsState, id: String, offer: LanWebuiOffer, onUpdated: (PluginDetail) -> Unit) {
    val scope = rememberCoroutineScope()
    var target by remember { mutableStateOf<Boolean?>(null) }
    var problem by remember { mutableStateOf<String?>(null) }
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
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
        if (target != null) CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
        problem?.let { Text(it, color = WontonTheme.colors.danger.toColor()) }
    }
}
