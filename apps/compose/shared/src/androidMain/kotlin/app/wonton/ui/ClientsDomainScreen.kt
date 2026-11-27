package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.stringResource
import app.wonton.ui.design.toColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.CaaState
import app.wonton.protocol.ClientsAction
import app.wonton.protocol.ClientsDomainState
import app.wonton.protocol.ClientsDomainViewModel
import app.wonton.protocol.ClientsProblem
import app.wonton.protocol.CredentialEntry
import app.wonton.protocol.LanRecordState
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.StepOptionRow
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.Copy
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/** 复制后打勾多久复位。和聊天里复制代码块一致。 */
private const val COPIED_RESET_MS = 1_500L
private const val DESEC_URL = "https://desec.io"

/**
 * “HTTPS 和域名”设置页（ADR-0089）：后缀、服务商、凭据、订户协议和证书状态。
 * 状态和动作都在客户端核心，这里只渲染 ViewModel、把表单值发回核心。
 */
@Composable
internal fun ClientsDomainPage(core: ClientCore, onBack: () -> Unit) {
    val view by remember(core) { core.clientsDomainUpdates() }
        .collectAsState(remember(core) { core.clientsDomain() })
    var rejection by remember { mutableStateOf<Problem?>(null) }
    var suffix by remember { mutableStateOf("") }
    var code by remember { mutableStateOf("") }
    var otherCredentials by remember { mutableStateOf("") }
    var credentials by remember { mutableStateOf<Map<String, String>>(emptyMap()) }
    var terms by remember { mutableStateOf(false) }
    val send = { action: Action -> rejection = core.send(action) }

    LaunchedEffect(core) { send(Action.Clients(ClientsAction.RefreshDomain)) }
    DisposableEffect(core) {
        onDispose { core.send(Action.Clients(ClientsAction.CancelDomain)) }
    }
    LaunchedEffect(view.suffix) { suffix = view.suffix.orEmpty() }
    LaunchedEffect(view.termsAccepted) { terms = view.termsAccepted }
    val scope = rememberCoroutineScope()

    WontonScreen(bar = {
        WontonPushedBar(stringResource(R.string.clients_domain_title), onBack)
    }) {
        Box(Modifier.fillMaxSize()) {
            when (val state = view.state) {
                ClientsDomainState.Loading -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                is ClientsDomainState.Failed -> ProblemView(state.problem.problem()) {
                    send(Action.Clients(ClientsAction.RefreshDomain))
                }
                ClientsDomainState.Ready, ClientsDomainState.Other -> DomainForm(
                    view = view,
                    suffix = suffix,
                    onSuffix = { suffix = it },
                    code = code,
                    onCode = { code = it },
                    otherCredentials = otherCredentials,
                    onOtherCredentials = { otherCredentials = it },
                    credentials = credentials,
                    onCredential = { name, value -> credentials = credentials + (name to value) },
                    terms = terms,
                    onTerms = { terms = it },
                    problem = rejection ?: view.problem?.problem(),
                    send = send,
                    onSave = {
                        val rejected = core.send(Action.Clients(ClientsAction.SaveDomain(
                            suffix = if (view.suffixEditable) suffix else null,
                            provider = if (view.otherProvider) code else view.selectedProvider,
                            credentials = view.credentialVars.mapNotNull { name ->
                                credentials[name].orEmpty().takeIf { it.isNotEmpty() }
                                    ?.let { CredentialEntry(name, it) }
                            },
                            otherCredentials = if (view.otherProvider) otherCredentials else "",
                            acceptTerms = terms,
                        )))
                        rejection = rejected
                        if (rejected == null) {
                            scope.launch {
                                val finished = core.clientsDomainUpdates().first { !it.saving }
                                if (finished.problem == null) onBack()
                            }
                        }
                    },
                )
            }
        }
    }
}

@Composable
private fun DomainForm(
    view: ClientsDomainViewModel,
    suffix: String,
    onSuffix: (String) -> Unit,
    code: String,
    onCode: (String) -> Unit,
    otherCredentials: String,
    onOtherCredentials: (String) -> Unit,
    credentials: Map<String, String>,
    onCredential: (String, String) -> Unit,
    terms: Boolean,
    onTerms: (Boolean) -> Unit,
    problem: Problem?,
    send: (Action) -> Unit,
    onSave: () -> Unit,
) {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        problem?.let { ProblemNotice(it, WontonIcon.Warning) }
        SuffixSection(view, suffix, onSuffix)
        ProviderSection(view, code, onCode, otherCredentials, onOtherCredentials, credentials, onCredential, send)
        TermsSection(terms, onTerms)
        CertificateSection(view, send)
        CaaSection(view)
        LanSection(view)
        if (view.desec) DesecSection()
        SaveButton(view.saving, onSave)
    }
}

@Composable
private fun DesecSection() {
    var expanded by remember { mutableStateOf(false) }
    val uri = LocalUriHandler.current
    WontonGroup {
        StepOptionRow(
            title = stringResource(R.string.clients_domain_desec),
            selected = expanded,
            onSelect = { expanded = !expanded },
            trailing = {
                WontonIconButton(
                    icon = WontonIcon.Forward,
                    label = stringResource(R.string.common_details),
                    onClick = { expanded = !expanded },
                )
            },
        )
        if (expanded) {
            SupportingText(
                stringResource(R.string.clients_domain_desec_steps),
                modifier = Modifier.padding(WontonTheme.spacing.s12),
            )
            LinkButton("desec.io") { uri.openUri(DESEC_URL) }
        }
    }
}

@Composable
internal fun RecordRow(record: String) {
    val colors = WontonTheme.colors
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    var copied by remember(record) { mutableStateOf(false) }
    LaunchedEffect(copied) {
        if (copied) {
            delay(COPIED_RESET_MS)
            copied = false
        }
    }
    Row(
        Modifier.fillMaxWidth()
            .background(colors.surfaceSubtle.toColor(), RoundedCornerShape(WontonTheme.radii.control))
            .padding(start = WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        SelectionContainer(Modifier.weight(1f)) {
            Text(
                record,
                style = WontonTheme.typography.supporting,
                fontFamily = FontFamily.Monospace,
                color = colors.textPrimary.toColor(),
            )
        }
        WontonIconButton(
            icon = if (copied) WontonIcon.Check else WontonIcon.Copy,
            label = stringResource(if (copied) R.string.clients_domain_copied else R.string.clients_domain_copy),
            onClick = {
                scope.launch {
                    clipboard.setClipEntry(
                        androidx.compose.ui.platform.ClipEntry(
                            android.content.ClipData.newPlainText(record, record),
                        ),
                    )
                }
                copied = true
            },
        )
    }
}

@Composable
internal fun LinkButton(text: String, onClick: () -> Unit) {
    Text(
        text,
        Modifier.clickable(role = Role.Button, onClick = onClick)
            .padding(WontonTheme.spacing.s12),
        style = WontonTheme.typography.supporting,
        color = WontonTheme.colors.accentInk.toColor(),
    )
}

@Composable
internal fun SupportingText(
    text: String,
    tint: androidx.compose.ui.graphics.Color? = null,
    modifier: Modifier = Modifier,
) {
    Text(
        text,
        modifier,
        style = WontonTheme.typography.supporting,
        color = tint ?: WontonTheme.colors.textSecondary.toColor(),
    )
}

internal fun caaText(state: CaaState): Int = when (state) {
    CaaState.BOUND -> R.string.clients_domain_caa_bound
    CaaState.MISSING -> R.string.clients_domain_caa_missing
    CaaState.MISMATCH -> R.string.clients_domain_caa_mismatch
    else -> R.string.clients_domain_caa_unknown
}

internal fun formatInstant(ms: Long): String =
    DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm")
        .withZone(ZoneId.systemDefault())
        .format(Instant.ofEpochMilli(ms))
