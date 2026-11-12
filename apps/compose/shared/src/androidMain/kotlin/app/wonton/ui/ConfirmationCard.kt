package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.heightIn
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
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
import app.wonton.protocol.ConfirmationCategory
import app.wonton.protocol.CardAudience
import app.wonton.protocol.ConfirmationTool
import app.wonton.protocol.ConfirmationToolImpact
import app.wonton.protocol.ConfirmationState
import app.wonton.protocol.ConfirmationView
import app.wonton.protocol.InstalledPlugin
import app.wonton.protocol.MyAppRef
import app.wonton.protocol.SettingValue
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

/** 核心给出卡片内容和风险类别；壳本地化固定提示，按钮状态仍由核心决定。 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ConfirmationCardView(view: ConfirmationView, onAction: (Action) -> Unit) {
    val card = view.card
    val colors = WontonTheme.colors
    var secrets by remember(card.id) { mutableStateOf<Map<String, String>>(emptyMap()) }
    var settings by remember(card.id) { mutableStateOf<Map<String, SettingValue?>>(emptyMap()) }
    var drafts by remember(card.id) { mutableStateOf<Map<String, String>>(emptyMap()) }
    var shown by remember(card.id) { mutableStateOf(false) }
    CardSecureScreen(card, secrets, shown)
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
        CardToolCallSection(card.category, card.id)
        if (card.secretInputs.isNotEmpty()) {
            SecretInputsSection(
                card.secretInputs,
                secrets,
                card.state == ConfirmationState.Pending,
                onShownChange = { shown = it },
            ) { name, value -> secrets = secrets + (name to value) }
        }
        card.settings?.fields?.forEach { field ->
            SettingsField(
                field, effectiveValue(field, settings), textDraft(field, settings, drafts), field.problem,
                disabled = card.state != ConfirmationState.Pending || isUnsupported(field, settings),
                missing = isMissingRequired(field, settings),
                onEdit = { name, value -> settings = settings + (name to value) },
                onDraft = { edited, text ->
                    drafts = drafts + (edited.name to text)
                    settings = settings + (edited.name to draftValue(edited, text))
                },
            )
        }
        card.answeredBy
            ?.takeIf { card.audience == CardAudience.ADMINS }
            ?.let { answeredBy ->
                Text(
                    stringResource(R.string.confirmation_card_answered_by, answeredBy),
                    style = WontonTheme.typography.supporting,
                )
            }
        CardState(card.state)
        view.window?.let { window -> CardWindow(window) { onAction(Action.StopConfirmationWindow(card.id)) } }
        view.problem?.let { ProblemRow(it.problem) }
        // 卡片变成成功态、且插件已在已装列表里时，给“打开 / 放到底栏”（ADR-0033）。查不到（已卸载、列表还没
        // 刷新）时如实不显示。
        CardInstalledActions(card, onAction)
        if (card.choices.isNotEmpty()) {
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
                verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
            ) {
                card.choices.forEach { choice ->
                    ChoiceButton(
                        choice,
                        view,
                        secretsReady = cardInputsReady(card, secrets, settings),
                    ) {
                        onAction(
                            Action.AnswerConfirmation(
                                card.id,
                                choice.answer,
                                choice.choiceId,
                                secretValues(card.secretInputs, secrets),
                                cardSettingsValues(settings),
                                card.devices.filter { it.chosen }.map { it.id },
                            ),
                        )
                    }
                }
            }
        }
    }
}

private fun cardInputsReady(
    card: ConfirmationCard,
    secrets: Map<String, String>,
    settings: Map<String, SettingValue?>,
): Boolean = !missingRequiredSecret(card.secretInputs, secrets) && card.settings?.fields.orEmpty().none {
    isUnsupported(it, settings) || isMissingRequired(it, settings)
}

private fun cardSettingsValues(settings: Map<String, SettingValue?>): Map<String, SettingValue> =
    settings.mapNotNull { (name, value) -> value?.let { name to it } }.toMap()

@Composable
private fun CardToolCallSection(category: ConfirmationCategory?, cardId: String) {
    val tool = (category as? ConfirmationCategory.ToolCall)?.tool ?: return
    CardToolSummary(tool)
    key(cardId) {
        ProblemDetail(
            text = "${tool.name}\n\n${tool.arguments}",
            modifier = Modifier.heightIn(min = WontonTheme.size.target),
        )
    }
}

@Composable
private fun CardToolSummary(tool: ConfirmationTool) {
    val colors = WontonTheme.colors
    val style = WontonTheme.typography.supporting
    Text(
        stringResource(R.string.confirmation_card_tool_source, tool.pluginTitle),
        style = style,
        color = colors.textSecondary.toColor(),
    )
    val warning = when (val impact = tool.impact) {
        ConfirmationToolImpact.ReadOnly -> null
        ConfirmationToolImpact.ChangesData -> stringResource(R.string.confirmation_card_tool_changes_data)
        is ConfirmationToolImpact.Destructive -> impact.target?.let {
            stringResource(R.string.confirmation_card_tool_destructive_target, it)
        } ?: stringResource(R.string.confirmation_card_tool_destructive_unknown)
        ConfirmationToolImpact.Other -> stringResource(R.string.confirmation_card_tool_unknown_impact)
    }
    warning?.let {
        val tint = if (tool.impact is ConfirmationToolImpact.Destructive) colors.danger else colors.warning
        Text(
            it,
            style = style,
            color = tint.toColor(),
        )
    }
    if (tool.runsAsRoot) {
        Text(
            stringResource(R.string.confirmation_card_tool_root),
            style = style,
            color = colors.warning.toColor(),
        )
    }
}

/** 已装插件可整页打开，也可加入“我的应用”；多页面复用应用选择页。卡片成功且插件还在已装列表里才显示。 */
@Composable
private fun CardInstalledActions(card: ConfirmationCard, onAction: (Action) -> Unit) {
    if (card.state !is ConfirmationState.Succeeded) return
    val tabs = LocalPages.current
    val installed = tabs.plugins.viewModel.installed?.plugins?.firstOrNull { plugin -> plugin.id == card.pluginId }
        ?: return
    var showsPages by remember(installed.id) { mutableStateOf(false) }
    val choices = installed.tabChoices
    val added = choices.any { choice -> tabs.plugins.viewModel.sidebar.apps.any { it.tab == choice.tab } }
    if (choices.isNotEmpty()) {
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
            window.problem?.let { ProblemRow(it.problem) }
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
