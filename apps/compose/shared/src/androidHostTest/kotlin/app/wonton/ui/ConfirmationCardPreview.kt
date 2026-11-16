package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import app.wonton.protocol.Answer
import app.wonton.protocol.CardAudience
import app.wonton.protocol.CardChoice
import app.wonton.protocol.CardLine
import app.wonton.protocol.ConfirmationCard
import app.wonton.protocol.ConfirmationCategory
import app.wonton.protocol.ConfirmationState
import app.wonton.protocol.ConfirmationTool
import app.wonton.protocol.ConfirmationToolImpact
import app.wonton.protocol.ConfirmationView
import app.wonton.ui.design.WontonTheme

internal fun previewToolConfirmation(
    id: String = "tool-card",
    impact: ConfirmationToolImpact = ConfirmationToolImpact.Destructive(null),
) = ConfirmationView(
    card = ConfirmationCard(
        id = id,
        state = ConfirmationState.Pending,
        title = "Remove old backups",
        lines = listOf(CardLine("Free up space used by older backups.", false)),
        choices = listOf(
            CardChoice(Answer.APPROVE, "Continue", null),
            CardChoice(Answer.REJECT, "Cancel", null),
        ),
        expiresAtMs = 0u,
        window = null,
        pluginId = null,
        secretInputs = emptyList(),
        settings = null,
        requester = null,
        category = ConfirmationCategory.ToolCall(
            ConfirmationTool(
                name = "backup__remove_old",
                pluginTitle = "Backups",
                arguments = "{\n  \"archive_ids\": [\"backup-123\", \"backup-456\"]\n}",
                impact = impact,
                runsAsRoot = true,
            ),
        ),
        origin = null,
        audience = CardAudience.KEEPER,
        answeredBy = null,
        permissions = emptyList(),
        devices = emptyList(),
    ),
    sending = null,
    sendingChoiceId = null,
    problem = null,
    window = null,
)

@Preview(name = "Confirmation English", locale = "en", widthDp = 390, heightDp = 844)
@Preview(name = "Confirmation Simplified Chinese", locale = "zh-rCN", widthDp = 390, heightDp = 844)
@Preview(name = "Confirmation Traditional Chinese", locale = "zh-rTW", widthDp = 390, heightDp = 844)
@Preview(name = "Confirmation Japanese", locale = "ja", widthDp = 390, heightDp = 844)
@Composable
internal fun ConfirmationCardPreview() {
    PreviewFrame {
        Column(
            Modifier.padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        ) {
            ConfirmationCardView(previewToolConfirmation()) {}
        }
    }
}

/** Interactive preview exercises the API-record entry and the complete confirmation sheet. */
@Preview(name = "Room confirmation entry", locale = "en", widthDp = 390, heightDp = 844)
@Composable
internal fun RoomConfirmationEntryPreview() {
    var opened by remember { mutableStateOf<String?>(null) }
    val cards = remember { listOf(previewToolConfirmation()) }
    PreviewFrame {
        RoomConfirmationRow(cards) { opened = it }
        RoomConfirmationSheet(cards, opened, onDismiss = { opened = null }, onAction = {})
    }
}
