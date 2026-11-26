package app.wonton.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.CaaState
import app.wonton.protocol.CertificateState
import app.wonton.protocol.ClientsAction
import app.wonton.protocol.ClientsDomainViewModel
import app.wonton.protocol.LanRecordState
import app.wonton.protocol.SecretInput
import app.wonton.ui.design.StepOptionRow
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

private const val CPA_URL = "https://letsencrypt.org/repository/"

@Composable
internal fun SuffixSection(view: ClientsDomainViewModel, suffix: String, onSuffix: (String) -> Unit) {
    val colors = WontonTheme.colors
    DetailSection(stringResource(R.string.clients_domain_suffix)) {
        if (view.suffixEditable) {
            WontonTextField(
                value = suffix,
                onValueChange = onSuffix,
                label = stringResource(R.string.clients_domain_suffix),
                enabled = !view.saving,
            )
            if (!view.suffix.isNullOrEmpty()) {
                SupportingText(stringResource(R.string.clients_domain_suffix_note))
            }
        } else {
            view.suffix?.let {
                SelectionContainer { SupportingText(it, colors.textPrimary.toColor()) }
            }
        }
    }
}

@Composable
internal fun ProviderSection(
    view: ClientsDomainViewModel,
    code: String,
    onCode: (String) -> Unit,
    otherCredentials: String,
    onOtherCredentials: (String) -> Unit,
    credentials: Map<String, String>,
    onCredential: (String, String) -> Unit,
    send: (Action) -> Unit,
) {
    val uri = LocalUriHandler.current
    DetailSection(stringResource(R.string.clients_domain_provider)) {
        view.providers.forEach { provider ->
            val providerCode = provider.code
            if (providerCode == null) {
                StepOptionRow(
                    title = stringResource(R.string.clients_domain_other),
                    selected = view.otherProvider,
                    onSelect = { send(Action.Clients(ClientsAction.SelectDomainProvider(null))) },
                )
            } else {
                StepOptionRow(
                    title = provider.name,
                    selected = view.selectedProvider == providerCode,
                    onSelect = { send(Action.Clients(ClientsAction.SelectDomainProvider(providerCode))) },
                )
            }
        }
        if (view.otherProvider) {
            WontonTextField(
                value = code,
                onValueChange = onCode,
                label = stringResource(R.string.clients_domain_other_code),
                enabled = !view.saving,
            )
            WontonTextField(
                value = otherCredentials,
                onValueChange = onOtherCredentials,
                placeholder = stringResource(R.string.clients_domain_other_hint),
                maxLines = 3,
                enabled = !view.saving,
            )
        } else if (view.credentialVars.isNotEmpty()) {
            SecretInputsSection(
                inputs = view.credentialVars.map { SecretInput(it, it, null, null, true) },
                values = credentials,
                enabled = !view.saving,
                onChange = onCredential,
            )
            if (view.credentialsSet) SupportingText(stringResource(R.string.clients_domain_credentials_set))
        }
        val signup = view.providers.firstOrNull { it.code == view.selectedProvider }?.signupUrl
        signup?.let {
            LinkButton(stringResource(R.string.clients_domain_signup)) { uri.openUri(it) }
        }
    }
}

@Composable
internal fun TermsSection(terms: Boolean, onTerms: (Boolean) -> Unit) {
    val uri = LocalUriHandler.current
    WontonGroup {
        StepOptionRow(
            title = stringResource(R.string.clients_domain_terms),
            selected = terms,
            onSelect = { onTerms(!terms) },
            multiple = true,
            modifier = Modifier.padding(WontonTheme.spacing.s12),
        )
        LinkButton(stringResource(R.string.clients_domain_terms_open)) { uri.openUri(CPA_URL) }
    }
}

@Composable
internal fun CertificateSection(view: ClientsDomainViewModel, send: (Action) -> Unit) {
    val colors = WontonTheme.colors
    DetailSection(stringResource(R.string.clients_domain_certificate)) {
        when (val state = view.certificate.state) {
            is CertificateState.Valid -> SupportingText(
                stringResource(
                    R.string.clients_domain_cert_valid,
                    formatInstant(state.notAfterMs.toLong()),
                ),
            )
            is CertificateState.Renewing -> SupportingText(
                stringResource(R.string.clients_domain_cert_renewing),
                colors.warning.toColor(),
            )
            is CertificateState.Expired -> SupportingText(
                stringResource(R.string.clients_domain_cert_expired),
                colors.danger.toColor(),
            )
            CertificateState.Issuing -> SupportingText(stringResource(R.string.clients_domain_cert_issuing))
            else -> SupportingText(stringResource(R.string.clients_domain_cert_none))
        }
        if (view.certificate.retryable) {
            WontonButton(
                text = stringResource(R.string.clients_domain_retry),
                onClick = { send(Action.Clients(ClientsAction.RetryCertificate)) },
                kind = WontonButtonKind.SECONDARY,
                enabled = !view.saving,
            )
        }
        view.certificate.details?.let { ProblemDetail(it) }
    }
}

@Composable
internal fun CaaSection(view: ClientsDomainViewModel) {
    val colors = WontonTheme.colors
    DetailSection(stringResource(R.string.clients_domain_caa)) {
        SupportingText(
            stringResource(caaText(view.caa)),
            if (view.caa == CaaState.MISMATCH) colors.warning.toColor() else null,
        )
        view.caaRecord?.let { RecordRow(it) }
    }
}

@Composable
internal fun LanSection(view: ClientsDomainViewModel) {
    val colors = WontonTheme.colors
    DetailSection(stringResource(R.string.clients_domain_lan)) {
        when (val state = view.lanRecord) {
            LanRecordState.Ok -> SupportingText(stringResource(R.string.clients_domain_lan_ok))
            LanRecordState.Missing -> SupportingText(
                stringResource(R.string.clients_domain_lan_missing),
                colors.warning.toColor(),
            )
            is LanRecordState.Wrong -> SupportingText(
                stringResource(R.string.clients_domain_lan_wrong, state.found),
                colors.warning.toColor(),
            )
            else -> SupportingText(stringResource(R.string.clients_domain_lan_unknown))
        }
        view.lanRecordName?.let { name ->
            val target = view.lanTarget ?: stringResource(R.string.clients_domain_lan_address)
            RecordRow("$name A $target")
        }
    }
}

@Composable
internal fun SaveButton(saving: Boolean, onSave: () -> Unit) {
    if (saving) {
        Box(
            Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target),
            contentAlignment = Alignment.Center,
        ) { CircularProgressIndicator(Modifier.size(24.dp)) }
    } else {
        WontonButton(
            text = stringResource(R.string.common_save),
            onClick = onSave,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}
