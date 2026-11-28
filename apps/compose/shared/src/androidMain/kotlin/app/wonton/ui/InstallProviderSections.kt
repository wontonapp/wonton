package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.client.deviceChoices
import app.wonton.client.toggleDevice
import app.wonton.protocol.CapabilityProvider
import app.wonton.protocol.CoreException
import app.wonton.protocol.InstallPreview
import app.wonton.protocol.InstallProvider
import app.wonton.protocol.SelectedProvider
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonRadioRow
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Blocked
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

/**
 * 确认页上缺的必需能力、一起装的提供方和缺的可选能力（ADR-0053）：提供方单独列一节权限、密钥和设备，
 * 高权限的标出来。
 */
@Composable
fun InstallProviderSections(
    preview: InstallPreview,
    plugins: PluginsState,
    onUpdate: (InstallPreview) -> Unit,
    onFillProfile: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    var switching by remember(preview.id) { mutableStateOf<String?>(null) }
    var problem by remember(preview.id) { mutableStateOf<Problem?>(null) }
    ConnectedSection(preview, onUpdate)
    preview.installProviders.forEach { section ->
        val wrong = stringResource(R.string.install_provider_wrong, section.capability)
        CapabilitySection(section, preview, switching, onUpdate, onFillProfile) { candidate ->
            switching = candidate.id
            scope.launch {
                try {
                    val fresh = plugins.core.previewInstall(candidate.source)
                    if (fresh.id != candidate.id) {
                        problem = Problem(wrong)
                    } else {
                        problem = null
                        onUpdate(preview.withSelected(section.capability) {
                            it.copy(provider = candidate, preview = fresh, secrets = emptyMap())
                        })
                    }
                } catch (failure: CoreException) {
                    problem = failure.problem
                } finally {
                    switching = null
                }
            }
        }
    }
    if (preview.missingWants.isNotEmpty()) WantsSection(preview)
    problem?.let { ProblemNotice(it) }
}

@Composable
internal fun SelectedProviderSection(
    consumer: String,
    capability: String,
    selected: SelectedProvider,
    preview: InstallPreview,
    onUpdate: (InstallPreview) -> Unit,
    onFillProfile: () -> Unit,
) {
    val colors = WontonTheme.colors
    Column(
        Modifier.padding(WontonTheme.spacing.s12)
            .background(colors.surfaceSubtle.toColor(), RoundedCornerShape(WontonTheme.radii.card)),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        FlowRow(
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        ) {
            Tag(selected.preview.trust.label, selected.preview.trust.tint)
            (selected.preview.provider ?: selected.preview.author?.name)?.let {
                Text(it, style = WontonTheme.typography.caption, color = colors.textSecondary.toColor())
            }
            if (selected.provider.highPrivilege) {
                Tag(stringResource(R.string.install_provider_high_privilege), colors.warning)
            }
        }
        if (selected.provider.highPrivilege) {
            Text(
                stringResource(
                    R.string.install_provider_connects,
                    consumer,
                    selected.provider.title,
                    capability,
                ),
                style = WontonTheme.typography.supporting,
                color = colors.warning.toColor(),
            )
        }
        selected.preview.blocked?.let { WontonNotice(WontonIcon.Blocked, it) }
        selected.preview.missingProfile?.let {
            WontonNotice(WontonIcon.Info, it, isError = false) {
                WontonButton(stringResource(R.string.install_fill_profile), onFillProfile)
            }
        }
        if (selected.preview.notices.isNotEmpty()) NoticeLines(selected.preview.notices)
        if (selected.preview.permissions.isNotEmpty()) {
            Text(
                stringResource(R.string.install_provider_permission_title, selected.provider.title),
                style = WontonTheme.typography.sectionTitle,
                color = colors.textPrimary.toColor(),
            )
            selected.preview.permissions.forEach { line ->
                PermissionRow(line) {
                    if (line.optional) {
                        WontonSwitch(line.enabled, { on ->
                            onUpdate(preview.withPermission(capability, line.permission, on))
                        }, line.supported)
                    }
                }
            }
        }
        if (selected.preview.secretInputs.isNotEmpty()) {
            Text(
                stringResource(R.string.install_provider_secret_title, selected.provider.title),
                style = WontonTheme.typography.sectionTitle,
                color = colors.textPrimary.toColor(),
            )
            SecretInputsSection(
                selected.preview.secretInputs,
                selected.secrets,
                onChange = { name, value -> onUpdate(preview.withSecret(capability, name, value)) },
            )
        }
        deviceChoices(selected.preview.devices.orEmpty(), selected.preview.permissions)?.let { section ->
            DeviceChoicesSection(section) { id ->
                onUpdate(preview.withDevices(capability, id))
            }
        }
        selected.preview.lanWebui?.let { offer ->
            LanWebuiToggle(offer, selected.preview.lanWebui?.enabled == true, enabled = true) { on ->
                onUpdate(preview.withLan(capability, on))
            }
        }
    }
}

private fun InstallPreview.withSelected(
    capability: String,
    change: (SelectedProvider) -> SelectedProvider,
): InstallPreview = copy(installProviders = installProviders.map { section ->
    if (section.capability != capability) section
    else section.copy(selected = section.selected?.let(change))
})

private fun InstallPreview.withPermission(capability: String, permission: String, on: Boolean) =
    withSelected(capability) { selected ->
        selected.copy(preview = selected.preview.copy(permissions = selected.preview.permissions.map {
            if (it.permission == permission) it.copy(enabled = on) else it
        }))
    }

private fun InstallPreview.withSecret(capability: String, name: String, value: String) =
    withSelected(capability) { selected ->
        selected.copy(secrets = if (value.isEmpty()) selected.secrets - name else selected.secrets + (name to value))
    }

private fun InstallPreview.withDevices(capability: String, id: String) =
    withSelected(capability) { selected ->
        selected.copy(preview = selected.preview.copy(
            devices = toggleDevice(selected.preview.devices.orEmpty(), id),
        ))
    }

internal fun InstallPreview.withConnected(id: String, on: Boolean): InstallPreview = copy(
    connectedPlugins = if (on) connectedPlugins + id else connectedPlugins - id,
)

private fun InstallPreview.withLan(capability: String, on: Boolean) =
    withSelected(capability) { selected ->
        selected.copy(preview = selected.preview.copy(lanWebui = selected.preview.lanWebui?.copy(enabled = on)))
    }
