package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.res.stringResource
import app.wonton.client.dependencyNextStep
import app.wonton.protocol.Action
import app.wonton.protocol.CapabilityProviderRef
import app.wonton.protocol.ConsumerCapabilitySetting
import app.wonton.protocol.CoreException
import app.wonton.protocol.DependencyNextStep
import app.wonton.protocol.PluginAction
import app.wonton.protocol.PluginDetail
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonRadioRow
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

/** 详情页的依赖异常（ADR-0053）：按类别说清发生了什么和下一步。 */
@Composable
fun PluginDependencyHints(
    detail: PluginDetail,
    plugins: PluginsState,
    onUpdate: (PluginDetail) -> Unit,
) {
    val scope = rememberCoroutineScope()
    var problem by remember(detail.id) { mutableStateOf<Problem?>(null) }
    detail.hints.forEach { hint ->
        when (dependencyNextStep(hint)) {
            DependencyNextStep.START_PROVIDER -> hint.provider?.let { provider ->
                WontonNotice(
                    WontonIcon.Warning,
                    stringResource(R.string.plugin_dependency_stopped, provider.title),
                    isError = false,
                ) {
                    WontonButton(
                        stringResource(R.string.plugin_dependency_start, provider.title),
                        { plugins.send(Action.Plugins(PluginAction.StartPlugin(provider.id))) },
                        kind = WontonButtonKind.SECONDARY,
                    )
                }
            }
            DependencyNextStep.CONNECT_PROVIDER -> hint.provider?.let { provider ->
                val capability = hint.capability.orEmpty()
                WontonNotice(
                    WontonIcon.Info,
                    stringResource(R.string.plugin_dependency_not_connected, provider.title, capability),
                    isError = false,
                ) {
                    WontonButton(stringResource(R.string.plugin_dependency_connect), {
                        scope.launch {
                            try {
                                onUpdate(plugins.core.setPluginCapability(detail.id, capability, provider.id))
                                problem = null
                            } catch (failure: CoreException) {
                                problem = failure.problem
                            }
                        }
                    }, kind = WontonButtonKind.SECONDARY)
                }
            }
            DependencyNextStep.OPEN_STORE -> WontonNotice(
                WontonIcon.Info,
                stringResource(R.string.plugin_dependency_missing, hint.capability.orEmpty()),
                isError = false,
                lines = listOf(stringResource(R.string.plugin_dependency_find_in_store)),
            )
            null -> Unit
        }
    }
    problem?.let { ProblemNotice(it) }
}

/** 装着、主人能改时给每种能力选提供方（ADR-0053）：跟全局默认，或者单独指定一个。 */
@Composable
fun PluginCapabilityChoices(
    detail: PluginDetail,
    plugins: PluginsState,
    onUpdate: (PluginDetail) -> Unit,
) {
    val colors = WontonTheme.colors
    val scope = rememberCoroutineScope()
    var problem by remember(detail.id) { mutableStateOf<Problem?>(null) }
    var busy by remember(detail.id) { mutableStateOf<String?>(null) }
    DetailSection(stringResource(R.string.plugin_capabilities_title)) {
        detail.capabilityProviders.forEach { setting ->
            CapabilityChoice(setting, busy != null) { provider ->
                busy = setting.capability
                scope.launch {
                    try {
                        onUpdate(plugins.core.setPluginCapability(detail.id, setting.capability, provider))
                        problem = null
                    } catch (failure: CoreException) {
                        problem = failure.problem
                    } finally {
                        busy = null
                    }
                }
            }
        }
        problem?.let { ProblemNotice(it) }
    }
}

@Composable
private fun CapabilityChoice(
    setting: ConsumerCapabilitySetting,
    busy: Boolean,
    choose: (String?) -> Unit,
) {
    val colors = WontonTheme.colors
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                setting.capability, style = WontonTheme.typography.body,
                color = colors.textPrimary.toColor(),
            )
            if (setting.thirdParty) {
                Tag(stringResource(R.string.install_provider_third_party), colors.textSecondary)
            }
        }
        WontonRadioRow(
            stringResource(R.string.plugin_capability_follow_default),
            setting.selected == null,
            !busy,
            { choose(null) },
        )
        setting.providers.forEach { provider ->
            WontonRadioRow(
                provider.title,
                setting.selected == provider.id,
                !busy,
                { choose(provider.id) },
                value = if (provider.highPrivilege) {
                    stringResource(R.string.install_provider_high_privilege)
                } else {
                    null
                },
                valueTint = if (provider.highPrivilege) colors.warning else null,
            )
        }
    }
}

/** 卸载被在用的依赖挡住（ADR-0053）：列出还在用的插件，给出“一起卸载”或“先停它们”。 */
@Composable
fun UninstallDependents(
    plugins: PluginsState,
    title: String,
    id: String,
    dependents: List<CapabilityProviderRef>,
) {
    WontonNotice(
        WontonIcon.Warning,
        stringResource(R.string.plugin_uninstall_in_use, title),
        isError = false,
        lines = dependents.map { it.title },
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            WontonButton(stringResource(R.string.plugin_uninstall_together), {
                plugins.send(Action.Plugins(PluginAction.UninstallPlugin(
                    id = id,
                    deleteData = false,
                    providers = emptyList(),
                    dependents = dependents.map { it.id },
                )))
            }, kind = WontonButtonKind.SECONDARY)
            WontonButton(stringResource(R.string.plugin_uninstall_stop_dependents), {
                dependents.forEach { dependent ->
                    plugins.send(Action.Plugins(PluginAction.StopPlugin(dependent.id)))
                }
            }, kind = WontonButtonKind.SECONDARY)
        }
    }
}
