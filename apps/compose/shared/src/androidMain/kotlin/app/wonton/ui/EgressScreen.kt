package app.wonton.ui

import android.icu.text.ListFormatter
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
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
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.PluginAction
import app.wonton.protocol.CoreException
import app.wonton.protocol.EgressConfirmation
import app.wonton.protocol.EgressCurrent
import app.wonton.protocol.EgressPathState
import app.wonton.protocol.EgressProxySource
import app.wonton.protocol.LanClient
import app.wonton.protocol.LanWebuiOffer
import app.wonton.protocol.PluginDetail
import app.wonton.protocol.Rgb
import app.wonton.ui.design.WontonRadioRow
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.wontonContentPadding
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
    LaunchedEffect(Unit) { plugins.send(Action.Plugins(PluginAction.RefreshEgressProxy)) }
    // 核心给的值变了才覆盖输入框：保存失败时主人填的还在。
    var address by remember(current?.address, current?.usePlugin) { mutableStateOf(current?.address.orEmpty()) }
    var usePlugin by remember(current?.address, current?.usePlugin) { mutableStateOf(current?.usePlugin == true) }
    var directCidrs by remember(current?.directCidrs) { mutableStateOf(current?.directCidrs.orEmpty()) }
    val selectedPlugin = current?.settings?.provider != null && usePlugin
    val save = { target: String, plugin: Boolean ->
        plugins.send(
            Action.Plugins(
                PluginAction.SaveEgressProxy(
                    address = target,
                    directCidrs = directCidrs,
                    usePlugin = plugin,
                ),
            ),
        )
    }
    val bar = @Composable {
        WontonPushedBar(stringResource(R.string.egress_title), onBack) {
            if (egress.saving) {
                CircularProgressIndicator(
                    Modifier.padding(end = WontonTheme.spacing.s12).size(WontonTheme.size.icon),
                    strokeWidth = 2.dp,
                )
            } else {
                Box(Modifier.padding(end = WontonTheme.spacing.s12)) {
                    WontonSmallButton(
                        stringResource(R.string.common_save),
                        { save(if (selectedPlugin) "" else address, selectedPlugin) },
                        current != null,
                    )
                }
            }
        }
    }
    WontonScreen(bar = bar) {
        Box(Modifier.fillMaxSize()) {
            val problem = egress.loadProblem
            when {
                current != null -> Column(
                    Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
                    verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
                ) {
                    egress.saveProblem?.let { ProblemNotice(it.saveProblem) }
                    problem?.let { ProblemNotice(it.problem) }
                    AddressField(current, address, selectedPlugin, !egress.saving, { usePlugin = it }) { address = it }
                    val plugin = current.pluginPath
                    val core = current.corePath
                    if (plugin != null && core != null) {
                        WontonGroup {
                            EgressPathRow(stringResource(R.string.egress_path_apps), plugin)
                            WontonSeparator()
                            EgressPathRow(stringResource(R.string.egress_path_home_server), core)
                        }
                    }
                    RangesField(current, directCidrs, !egress.saving) { directCidrs = it }
                    if (!current.addressLocked &&
                        (current.settings.url != null || current.settings.source == EgressProxySource.PLUGIN)
                    ) {
                        WontonButton(
                            stringResource(R.string.egress_turn_off),
                            { save("", false) },
                            Modifier.fillMaxWidth(),
                            WontonButtonKind.DANGER,
                            enabled = !egress.saving,
                        )
                    }
                }
                problem != null -> ProblemView(problem.problem) {
                    plugins.send(Action.Plugins(PluginAction.RefreshEgressProxy))
                }
                else -> LoadingView()
            }
        }
    }
    egress.confirmation?.let { RestartDialog(it, plugins) }
}

@Composable
private fun RestartDialog(confirmation: EgressConfirmation, plugins: PluginsState) {
    val dismiss = { plugins.send(Action.Plugins(PluginAction.DismissEgressConfirmation)) }
    val apps = ListFormatter.getInstance(LocalConfiguration.current.locales[0])
        .format(confirmation.restarts.map { it.title })
    WontonDialog(
        stringResource(
            if (confirmation.cancelling) R.string.egress_restart_cancel_title else R.string.egress_restart_set_title,
        ),
        stringResource(R.string.egress_restart_message, apps),
        dismiss,
        listOf(
            WontonDialogAction(stringResource(R.string.common_cancel), onClick = dismiss),
            WontonDialogAction(stringResource(R.string.egress_restart_confirm), WontonButtonKind.PRIMARY) {
                plugins.send(Action.Plugins(PluginAction.ConfirmEgressProxy))
            },
        ),
    )
}

@Composable
private fun AddressField(
    current: EgressCurrent,
    value: String,
    usePlugin: Boolean,
    enabled: Boolean,
    onModeChange: (Boolean) -> Unit,
    onChange: (String) -> Unit,
) {
    DetailSection(stringResource(R.string.egress_address)) {
        current.settings.provider?.let { provider ->
            Column(
                Modifier.selectableGroup(),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                val canSelect = enabled && !current.addressLocked
                WontonRadioRow(
                    stringResource(R.string.egress_address_use_provider, provider.title), usePlugin, canSelect,
                    { onModeChange(true) },
                )
                WontonRadioRow(
                    stringResource(R.string.egress_address_enter), !usePlugin, canSelect,
                    { onModeChange(false) },
                )
            }
        }
        if (!usePlugin) {
            WontonTextField(
                value, onChange, Modifier.fillMaxWidth(),
                placeholder = "http://192.0.2.10:7890", label = stringResource(R.string.egress_address),
                enabled = enabled && !current.addressLocked,
                keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Uri),
            )
        }
        when (current.settings.source) {
            EgressProxySource.ENV -> Secondary(stringResource(R.string.egress_source_env))
            EgressProxySource.DETECTED -> Secondary(stringResource(R.string.egress_source_detected))
            EgressProxySource.PLUGIN -> current.settings.provider?.let {
                Secondary(stringResource(R.string.egress_source_provider, it.title))
            }
            EgressProxySource.UNSET,
            EgressProxySource.SETTINGS,
            EgressProxySource.UNKNOWN,
            -> Unit
        }
    }
}

@Composable
private fun RangesField(current: EgressCurrent, value: String, enabled: Boolean, onChange: (String) -> Unit) {
    DetailSection(stringResource(R.string.egress_ranges)) {
        WontonTextField(
            value,
            onChange,
            Modifier.fillMaxWidth(),
            placeholder = "100.64.0.0/10",
            label = stringResource(R.string.egress_ranges),
            enabled = enabled,
            keyboardOptions = KeyboardOptions(autoCorrectEnabled = false),
        )
        if (current.settings.hostCidrs.isNotEmpty()) {
            Secondary(stringResource(R.string.egress_ranges_host, current.settings.hostCidrs.joinToString(", ")))
        }
        Secondary(stringResource(R.string.egress_ranges_note))
        current.settings.notices.forEach { Secondary(it, WontonTheme.colors.warning) }
    }
}

/** 一条路径通不通，不通时写原因。 */
@Composable
private fun EgressPathRow(title: String, state: EgressPathState) {
    val colors = WontonTheme.colors
    val (label, tint, message) = when (state) {
        is EgressPathState.Probing -> Triple(R.string.egress_path_probing, colors.textSecondary, null)
        is EgressPathState.Reachable -> Triple(R.string.egress_path_reachable, colors.success, null)
        is EgressPathState.Uncertain -> Triple(R.string.egress_path_uncertain, colors.warning, state.message)
        is EgressPathState.Failed -> Triple(R.string.egress_path_failed, colors.danger, state.message)
    }
    Column {
        WontonRow(title, stringResource(label), tint)
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
                Text(
                    stringResource(R.string.lan_webui_toggle),
                    style = WontonTheme.typography.body,
                    color = colors.textPrimary.toColor(),
                )
                Secondary(
                    pluralStringResource(
                        R.plurals.lan_webui_ports, offer.ports.size,
                        offer.ports.joinToString(", ") { it.port.toString() },
                    ),
                )
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
    var problem by remember { mutableStateOf<Problem?>(null) }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        LanWebuiToggle(offer, target ?: offer.enabled, enabled = target == null) { enabled ->
            target = enabled
            problem = null
            scope.launch {
                try {
                    onUpdated(plugins.core.setLanWebui(id, enabled))
                } catch (failure: CoreException) {
                    problem = failure.problem
                    runCatching { plugins.core.installedPlugin(id) }.onSuccess(onUpdated)
                }
                target = null
            }
        }
        if (target != null) CircularProgressIndicator(Modifier.size(WontonTheme.size.icon), strokeWidth = 2.dp)
        problem?.let { ProblemNotice(it) }
        offer.clientProblem?.let { WontonNotice(WontonIcon.Warning, it) }
        offer.client?.let { LanClientAccount(it) }
    }
}

/** 第三方客户端连这个插件用的地址和账号，可以选中复制。 */
@Composable
private fun LanClientAccount(client: LanClient) {
    SelectionContainer {
        Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            client.addresses.forEach { LanClientRow(stringResource(R.string.lan_client_address), it) }
            LanClientRow(stringResource(R.string.lan_client_username), client.username)
            LanClientRow(stringResource(R.string.lan_client_password), client.password)
        }
    }
}

@Composable
private fun LanClientRow(name: String, value: String) {
    val colors = WontonTheme.colors
    Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Text(name, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
        Text(value, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
    }
}
