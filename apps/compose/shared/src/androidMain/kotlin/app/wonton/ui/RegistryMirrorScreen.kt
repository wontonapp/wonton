package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.PluginFailure
import app.wonton.protocol.RegistryMirrorSettings
import app.wonton.protocol.RegistryMirrorViewModel
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

private class MirrorDraft(settings: RegistryMirrorSettings) {
    var enabled by mutableStateOf(settings.mirror != null)
    var choice by mutableStateOf(settings.mirror?.takeIf { it in settings.choices })
    var custom by mutableStateOf(settings.mirror != null && settings.mirror !in settings.choices)
    var address by mutableStateOf(settings.mirror?.takeIf { it !in settings.choices }.orEmpty())

    val target: String get() = if (!enabled) "" else if (custom) address else choice.orEmpty()
    val canSave: Boolean get() = !enabled || target.isNotBlank()
}

@Composable
fun RegistryMirrorPage(plugins: PluginsState, onBack: () -> Unit) {
    val mirror = plugins.viewModel.registryMirror
    val settings = mirror.settings
    val loadProblem = mirror.loadProblem
    LaunchedEffect(Unit) { plugins.send(Action.RefreshRegistryMirror) }
    // Probe, recommendation and choice-list updates must not replace an unfinished draft.
    val draft = remember(settings?.mirror, settings != null) { settings?.let { MirrorDraft(it) } }
    WontonScreen(bar = {
        WontonPushedBar(stringResource(R.string.registry_mirror_title), onBack) {
            Box(Modifier.padding(end = WontonTheme.spacing.s12)) {
                if (mirror.saving) MirrorSpinner()
                else WontonSmallButton(
                    stringResource(R.string.common_save),
                    { draft?.let { plugins.send(Action.SaveRegistryMirror(it.target)) } },
                    enabled = draft?.canSave == true,
                )
            }
        }
    }) {
        when {
            settings != null && draft != null -> MirrorForm(mirror, settings, draft, plugins)
            loadProblem != null -> ProblemView(loadProblem.problem) {
                plugins.send(Action.RefreshRegistryMirror)
            }
            else -> LoadingView()
        }
    }
}

@Composable
private fun MirrorForm(
    mirror: RegistryMirrorViewModel,
    settings: RegistryMirrorSettings,
    draft: MirrorDraft,
    plugins: PluginsState,
) {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        mirror.loadProblem?.let {
            ProblemNotice(it.problem)
            WontonSmallButton(
                stringResource(R.string.common_retry), { plugins.send(Action.RefreshRegistryMirror) }, !mirror.loading,
            )
        }
        mirror.saveProblem?.let {
            val problem = if (it.reason == PluginFailure.INVALID) {
                Problem(stringResource(R.string.registry_mirror_save_failed), detail = it.message)
            } else it.saveProblem
            ProblemNotice(problem)
        }
        mirror.testProblem?.let { ProblemNotice(it.problem) }
        if (settings.recommended && !draft.enabled) {
            WontonNotice(
                WontonIcon.Warning,
                stringResource(R.string.registry_mirror_recommended),
                isError = false,
            )
        }
        MirrorToggle(draft, enabled = !mirror.saving)
        if (draft.enabled) MirrorChoices(settings, mirror, draft)
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            WontonButton(
                stringResource(R.string.registry_mirror_test),
                { plugins.send(Action.TestRegistryMirrors) },
                kind = WontonButtonKind.SECONDARY,
                enabled = !mirror.testing && !mirror.saving,
            )
            if (mirror.testing) MirrorSpinner()
        }
        MirrorResults(mirror, settings, draft)
    }
}

@Composable
private fun MirrorResults(
    mirror: RegistryMirrorViewModel,
    settings: RegistryMirrorSettings,
    draft: MirrorDraft,
) {
    // A completed test with no direct probe is not the older settings probe.
    val tests = mirror.tests
    tests?.mirrors?.forEach { result ->
        val customAddress = when {
            draft.custom -> draft.address
            draft.choice !in settings.choices -> draft.choice
            else -> null
        }
        if (!draft.enabled || (result.mirror !in settings.choices && result.mirror != customAddress)) {
            RegistryProbeRow(result.mirror, result.probe)
        }
    }
    val direct = if (tests != null) tests.dockerHub else settings.dockerHub
    RegistryProbeRow(stringResource(R.string.registry_mirror_docker_hub_direct), direct)
}

@Composable
private fun MirrorToggle(draft: MirrorDraft, enabled: Boolean) {
    val title = stringResource(R.string.registry_mirror_title)
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(
                title, Modifier.weight(1f),
                style = WontonTheme.typography.body, color = WontonTheme.colors.textPrimary.toColor(),
            )
            WontonSwitch(
                draft.enabled, { draft.enabled = it }, enabled,
                Modifier.widthIn(min = WontonTheme.size.target).semantics { contentDescription = title },
            )
        }
        Text(
            stringResource(R.string.registry_mirror_risk),
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
    }
}

@Composable
private fun MirrorChoices(settings: RegistryMirrorSettings, mirror: RegistryMirrorViewModel, draft: MirrorDraft) {
    val removedChoice = !draft.custom && draft.choice != null && draft.choice !in settings.choices
    val customAddress = if (removedChoice) draft.choice.orEmpty() else draft.address
    Column(
        Modifier.selectableGroup(),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        settings.choices.forEach { address ->
            RegistryChoiceRow(
                address, selected = !draft.custom && draft.choice == address, enabled = !mirror.saving,
                probe = mirror.tests?.mirrors?.firstOrNull { it.mirror == address }?.probe,
            ) {
                draft.custom = false
                draft.choice = address
            }
        }
        RegistryChoiceRow(
            stringResource(R.string.registry_mirror_custom),
            selected = draft.custom || removedChoice,
            enabled = !mirror.saving,
            probe = mirror.tests?.mirrors?.firstOrNull { it.mirror == customAddress }?.probe,
        ) {
            if (removedChoice) draft.address = customAddress
            draft.custom = true
        }
        if (draft.custom || removedChoice) {
            WontonTextField(
                customAddress,
                {
                    draft.address = it
                    draft.custom = true
                },
                Modifier.fillMaxWidth(),
                label = stringResource(R.string.registry_mirror_custom_address),
                placeholder = "mirror.example.com:5000", enabled = !mirror.saving,
                keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Uri),
            )
        }
    }
}

@Composable
private fun MirrorSpinner() {
    CircularProgressIndicator(Modifier.size(WontonTheme.size.icon), strokeWidth = 2.dp)
}
