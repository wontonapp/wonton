package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import app.wonton.protocol.CapabilityProvider
import app.wonton.protocol.InstallPreview
import app.wonton.protocol.InstallProvider
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonRadioRow
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.icons.Blocked
import app.wonton.ui.icons.WontonIcon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.MissingCapability
import app.wonton.ui.R
import app.wonton.protocol.PluginDetail
import app.wonton.protocol.PluginFailure
import app.wonton.protocol.PluginTask
import app.wonton.protocol.PluginTaskState
import app.wonton.protocol.ThemeColors
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

/** 依赖异常、能力提供方选择和卸载被挡（ADR-0053）。 */
@Composable
internal fun DependencySections(
    detail: PluginDetail,
    plugins: PluginsState,
    task: PluginTask?,
    isInstalled: Boolean,
    isOwner: Boolean,
    onUpdate: (PluginDetail) -> Unit,
) {
    PluginDependencyHints(detail, plugins, onUpdate)
    PluginMissingCapabilities(detail)
    if (isInstalled && isOwner && detail.capabilityProviders.isNotEmpty()) {
        PluginCapabilityChoices(detail, plugins, onUpdate)
    }
    val dependents = (task?.state as? PluginTaskState.Failed)
        ?.takeIf { it.reason == PluginFailure.IN_USE }
        ?.let { task?.dependents.orEmpty() }
        .orEmpty()
    if (dependents.isNotEmpty()) {
        UninstallDependents(plugins, detail.title, detail.id, dependents)
    }
}


/** 详情页列出缺的必需和可选能力（ADR-0053）：商店详情不打开确认页也要看得到缺什么、有没有提供方。 */
@Composable
fun PluginMissingCapabilities(detail: PluginDetail) {
    val colors = WontonTheme.colors
    val requires = detail.missingRequires
    val wants = detail.missingWants
    if (requires.isEmpty() && wants.isEmpty()) return
    DetailSection(stringResource(R.string.plugin_dependencies_missing)) {
        requires.forEach { row(it, required = true, colors) }
        wants.forEach { row(it, required = false, colors) }
    }
}

@Composable
private fun row(missing: MissingCapability, required: Boolean, colors: ThemeColors) {
    Column {
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            Text(
                stringResource(
                    if (required) R.string.plugin_dependency_requires
                    else R.string.plugin_dependency_optional,
                    missing.capability,
                ),
                style = WontonTheme.typography.body,
                color = colors.textPrimary.toColor(),
            )
            if (missing.thirdParty) {
                Tag(stringResource(R.string.install_provider_third_party), colors.textSecondary)
            }
        }
        if (missing.providers.isEmpty()) {
            Text(
                stringResource(R.string.plugin_dependency_no_provider),
                style = WontonTheme.typography.supporting,
                color = (if (required) colors.warning else colors.textSecondary).toColor(),
            )
        } else {
            Text(
                stringResource(
                    R.string.plugin_dependency_provided_by,
                    missing.providers.map { it.title }.joinToString(", "),
                ),
                style = WontonTheme.typography.supporting,
                color = colors.textSecondary.toColor(),
            )
        }
    }
}

/**
 * 装的是提供方：已装并想用这个能力的插件（ADR-0053 第 3 条）。外层装提供方时勾选的才接上；
 * 随批安装的提供方内层带不了连接，只列出来。
 */
@Composable
internal fun ConnectedSection(preview: InstallPreview, onUpdate: (InstallPreview) -> Unit) {
    val colors = WontonTheme.colors
    // 升级时只有收 connect_plugins 的核（upgradeConnects）给开关；旧核知道想用它的应用，但收下请求
    // 也不认，升级页连这一节都不显示。安装页总是给开关，随批装的提供方内层带不了连接，只列出来。
    if (preview.upgradeFrom != null && !preview.upgradeConnects) return
    val direct = preview.wantingPlugins.isNotEmpty()
    val wanting = if (preview.wantingPlugins.isNotEmpty()) {
        preview.wantingPlugins.distinctBy { it.id }
    } else {
        preview.installProviders
            .flatMap { it.selected?.preview?.wantingPlugins.orEmpty() }
            .distinctBy { it.id }
    }
    if (wanting.isEmpty()) return
    DetailSection(stringResource(R.string.install_wants_connected_title)) {
        wanting.forEach { plugin ->
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    plugin.title,
                    Modifier.weight(1f),
                    style = WontonTheme.typography.body,
                    color = colors.textPrimary.toColor(),
                )
                if (plugin.highPrivilege) {
                    Tag(stringResource(R.string.install_provider_high_privilege), colors.warning)
                }
                if (direct) {
                    WontonSwitch(plugin.id in preview.connectedPlugins, { on ->
                        onUpdate(preview.withConnected(plugin.id, on))
                    }, true, Modifier.semantics { contentDescription = plugin.title })
                }
            }
        }
    }
}


@Composable
internal fun CapabilitySection(
    section: InstallProvider,
    preview: InstallPreview,
    switching: String?,
    onUpdate: (InstallPreview) -> Unit,
    onFillProfile: () -> Unit,
    onSelect: (CapabilityProvider) -> Unit,
) {
    val colors = WontonTheme.colors
    DetailSection(stringResource(R.string.install_required_capabilities)) {
        Row(
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                section.capability, style = WontonTheme.typography.itemTitle,
                color = colors.textPrimary.toColor(),
            )
            if (section.thirdParty) {
                Tag(stringResource(R.string.install_provider_third_party), colors.textSecondary)
            }
        }
        if (section.providers.isEmpty()) {
            WontonNotice(WontonIcon.Blocked, stringResource(R.string.install_provider_missing))
        }
        if (section.providers.size > 1) {
            section.providers.forEach { candidate ->
                CandidateRow(candidate, section.selected?.provider?.id, switching, onSelect)
            }
        }
        section.selected?.let { selected ->
            SelectedProviderSection(
                consumer = preview.title,
                capability = section.capability,
                selected = selected,
                preview = preview,
                onUpdate = onUpdate,
                onFillProfile = onFillProfile,
            )
        }
    }
}


@Composable
internal fun CandidateRow(
    candidate: CapabilityProvider,
    selected: String?,
    switching: String?,
    onSelect: (CapabilityProvider) -> Unit,
) {
    WontonRadioRow(
        candidate.title,
        selected == candidate.id,
        switching == null,
        { onSelect(candidate) },
        value = if (candidate.highPrivilege) {
            stringResource(R.string.install_provider_high_privilege)
        } else {
            null
        },
        valueTint = if (candidate.highPrivilege) WontonTheme.colors.warning else null,
    )
}


@Composable
internal fun WantsSection(preview: InstallPreview) {
    val colors = WontonTheme.colors
    DetailSection(stringResource(R.string.install_optional_capabilities)) {
        preview.missingWants.forEach { missing ->
            Row(
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(missing.capability, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
                if (missing.thirdParty) {
                    Tag(stringResource(R.string.install_provider_third_party), colors.textSecondary)
                }
            }
            Text(
                stringResource(R.string.install_wants_missing, missing.capability),
                style = WontonTheme.typography.supporting,
                color = colors.textSecondary.toColor(),
            )
        }
    }
}

