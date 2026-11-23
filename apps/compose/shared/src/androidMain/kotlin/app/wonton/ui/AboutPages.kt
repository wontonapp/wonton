package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import app.wonton.client.ClientCore
import app.wonton.protocol.LicenseGroup
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.ProblemDetail

/** 隐私说明：正文按段落排，段落和标题都由核心给。 */
@Composable
fun PrivacyPage(core: ClientCore, back: () -> Unit) {
    val sections = remember(core) { core.privacy() }
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.settings_privacy), back) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            sections.sections.forEach { section ->
                Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                    Text(
                        section.title,
                        style = WontonTheme.typography.supporting,
                        color = WontonTheme.colors.textSecondary.toColor(),
                    )
                    section.paragraphs.forEach { paragraph ->
                        Text(
                            paragraph,
                            style = WontonTheme.typography.body,
                            color = WontonTheme.colors.textPrimary.toColor(),
                        )
                    }
                }
            }
        }
    }
}

/** 开源许可：一个许可一组，组里是用到它的组件，组尾打开全文。 */
@Composable
fun LicensesPage(licenses: LicensesLoad, onOpen: (LicenseGroup) -> Unit, back: () -> Unit) {
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.settings_licenses), back) }) {
        val result = licenses.result ?: return@WontonScreen
        if (result.isFailure) {
            val reason = result.exceptionOrNull()?.let { it.message ?: it.toString() }
            LicensesProblem(reason, licenses.retry)
        } else {
            LazyColumn(
                Modifier.fillMaxSize().wontonContentPadding().padding(WontonTheme.spacing.pageInset),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
            ) {
                items(result.getOrThrow().licenses, key = { it.id }) { license ->
                    LicenseBlock(license, onOpen = { onOpen(license) })
                }
            }
        }
    }
}

@Composable
private fun LicensesProblem(detail: String?, retry: () -> Unit) {
    Column(
        Modifier.fillMaxSize().wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        Text(
            stringResource(R.string.settings_licenses_problem),
            style = WontonTheme.typography.body,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        // 原因按类别由核心按 App 的语言给，收进默认收起的详情。
        if (detail != null) ProblemDetail(detail)
        WontonSmallButton(stringResource(R.string.common_retry), retry)
    }
}

@Composable
private fun LicenseBlock(license: LicenseGroup, onOpen: () -> Unit) {
    WontonGroup {
        license.components.forEachIndexed { index, component ->
            if (index > 0) WontonSeparator()
            WontonRow(component.name, component.version, valueOneLine = true)
        }
        WontonSeparator()
        WontonRow(stringResource(R.string.settings_licenses_text), showsForward = true, onClick = onOpen)
    }
}

/** 一个许可的全文，等宽排版，长行跟着宽度折。 */
@Composable
fun LicenseTextPage(license: LicenseGroup, back: () -> Unit) {
    WontonScreen(bar = { WontonPushedBar(license.name, back) }) {
        Column(
            Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        ) {
            Text(
                license.text,
                Modifier.fillMaxWidth(),
                style = WontonTheme.typography.caption.copy(fontFamily = FontFamily.Monospace),
                color = WontonTheme.colors.textPrimary.toColor(),
            )
        }
    }
}
