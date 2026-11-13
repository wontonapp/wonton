package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.Answer
import app.wonton.protocol.CardChoice
import app.wonton.protocol.ConfirmationCard
import app.wonton.protocol.ConfirmationState
import app.wonton.protocol.ConfirmationView
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.MyAppRef
import app.wonton.protocol.WindowView
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.CheckCircleFill
import app.wonton.ui.icons.Error
import app.wonton.ui.icons.Help
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import java.text.DateFormat
import java.util.Date

/** 确认卡片：文字是家里的服务器写好的，按钮有没有、能不能点都由核心决定。 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ConfirmationCardView(view: ConfirmationView, onAction: (Action) -> Unit) {
    val card = view.card
    val colors = WontonTheme.colors
    var secrets by remember(card.id) { mutableStateOf<Map<String, String>>(emptyMap()) }
    var shown by remember(card.id) { mutableStateOf(false) }
    CardSecureScreen(card, secrets, shown)
    val tabs = LocalPages.current
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Text(card.title, style = WontonTheme.typography.itemTitle)
        card.lines.forEach { line ->
            Text(
                line.text,
                style = WontonTheme.typography.supporting,
                fontWeight = if (line.emphasis) FontWeight.SemiBold else FontWeight.Normal,
                color = (if (line.emphasis) colors.danger else colors.textPrimary).toColor(),
            )
        }
        if (card.secretInputs.isNotEmpty()) {
            SecretInputsSection(
                card.secretInputs,
                secrets,
                card.state == ConfirmationState.Pending,
                onShownChange = { shown = it },
            ) { name, value -> secrets = secrets + (name to value) }
        }
        CardState(card.state)
        view.window?.let { window -> CardWindow(window) { onAction(Action.StopConfirmationWindow(card.id)) } }
        view.problem?.let { problem -> StateLine(WontonIcon.Warning, problem.message, colors.danger) }
        // 卡片变成成功态、且插件已在已装列表里时，给“打开 / 放到底栏”（ADR-0033）。查不到（已卸载、列表还没
        // 刷新）时如实不显示。
        val installed = if (card.state is ConfirmationState.Succeeded) {
            card.pluginId?.let { id -> tabs.plugins.viewModel.installed?.plugins?.firstOrNull { it.id == id } }
        } else {
            null
        }
        installed?.let { CardInstalledActions(it, tabs, onAction) }
        if (card.choices.isNotEmpty()) {
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                card.choices.forEach { choice ->
                    ChoiceButton(
                        choice,
                        view,
                        secretsReady = !missingRequiredSecret(card.secretInputs, secrets),
                    ) {
                        onAction(
                            Action.AnswerConfirmation(
                                card.id,
                                choice.answer,
                                choice.choiceId,
                                secretValues(card.secretInputs, secrets),
                                emptyMap(),
                            ),
                        )
                    }
                }
            }
        }
    }
}

/** 卡片上已装插件的按钮：“打开”到第一项 tab；“放到底栏”走现有固定流程，已固定时不显示。 */
@Composable
private fun CardInstalledActions(installed: InstalledPlugin, tabs: PagesState, onAction: (Action) -> Unit) {
    var showsPages by remember(installed.id) { mutableStateOf(false) }
    val choices = installed.tabChoices
    val added = choices.any { choice -> tabs.plugins.viewModel.sidebar.apps.any { it.tab == choice.tab } }
    if (choices.isEmpty()) return
    FlowRow(
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        WontonButton(stringResource(R.string.tab_open), { onAction(Action.OpenPage(choices[0].tab)) })
        if (!added) {
            WontonButton(stringResource(R.string.myapp_add), {
                if (choices.size > 1) {
                    showsPages = true
                } else {
                    tabs.send(Action.AddMyApp(MyAppRef.Page(choices.single().tab), null, null))
                }
            })
        }
    }
    if (showsPages) MyAppChoiceSheet(installed) { showsPages = false }
}

@Composable
private fun CardState(state: ConfirmationState) {
    val style = WontonTheme.typography.supporting
    val colors = WontonTheme.colors
    when (state) {
        ConfirmationState.Pending -> Unit
        ConfirmationState.Running -> Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        ) {
            CircularProgressIndicator(Modifier.size(WontonTheme.size.iconTiny), strokeWidth = 2.dp)
            Text(stringResource(R.string.confirmation_card_running), style = style)
        }
        is ConfirmationState.Succeeded -> StateLine(WontonIcon.CheckCircleFill, state.message, colors.success)
        is ConfirmationState.Failed -> StateLine(
            WontonIcon.Error,
            stringResource(R.string.confirmation_card_failed_result, state.message),
            colors.danger,
        )
        is ConfirmationState.Uncertain -> StateLine(
            WontonIcon.Help,
            stringResource(R.string.confirmation_card_uncertain_result, state.message),
            colors.warning,
        )
        ConfirmationState.Rejected -> Text(
            stringResource(R.string.confirmation_card_rejected),
            style = style,
            color = colors.textSecondary.toColor(),
        )
        ConfirmationState.Expired -> Text(
            stringResource(R.string.confirmation_card_expired),
            style = style,
            color = colors.textSecondary.toColor(),
        )
    }
}

@Composable
private fun CardWindow(window: WindowView, onStop: () -> Unit) {
    val style = WontonTheme.typography.supporting
    val colors = WontonTheme.colors
    when (window) {
        is WindowView.Ended -> Text(window.note, style = style, color = colors.textSecondary.toColor())
        is WindowView.Active -> {
            val until = DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(window.expiresAtMs.toLong()))
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                Text(stringResource(R.string.confirmation_card_window_until, until), style = style)
                if (window.stopping) {
                    CircularProgressIndicator(Modifier.size(WontonTheme.size.iconTiny), strokeWidth = 2.dp)
                }
                WontonButton(
                    stringResource(R.string.confirmation_card_stop_window),
                    onStop,
                    kind = WontonButtonKind.SECONDARY,
                    enabled = !window.stopping,
                )
            }
            window.problem?.let { problem -> StateLine(WontonIcon.Warning, problem.message, colors.danger) }
        }
    }
}

/** 框里有值或点过“显示”、卡片还没答完时，这一屏不能截。 */
private fun cardHidesValues(card: ConfirmationCard, secrets: Map<String, String>, shown: Boolean): Boolean =
    card.secretInputs.isNotEmpty() && (shown || card.secretInputs.any { secrets[it.name]?.isNotEmpty() == true })

@Composable
private fun CardSecureScreen(card: ConfirmationCard, secrets: Map<String, String>, shown: Boolean) {
    if (card.state == ConfirmationState.Pending && cardHidesValues(card, secrets, shown)) SecureScreen()
}

@Composable
private fun ChoiceButton(
    choice: CardChoice,
    view: ConfirmationView,
    secretsReady: Boolean,
    onClick: () -> Unit,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
    ) {
        if (view.sending == choice.answer && view.sendingChoiceId == choice.choiceId) {
            CircularProgressIndicator(Modifier.size(WontonTheme.size.iconTiny), strokeWidth = 2.dp)
        }
        WontonButton(
            choice.label,
            onClick,
            kind = if (choice.answer == Answer.APPROVE) WontonButtonKind.PRIMARY else WontonButtonKind.SECONDARY,
            enabled = view.sending == null &&
                (choice.answer == Answer.REJECT || secretsReady),
        )
    }
}
