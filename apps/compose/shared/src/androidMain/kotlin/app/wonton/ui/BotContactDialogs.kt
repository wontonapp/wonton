package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import app.wonton.protocol.BotContact
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonRadioRow
import app.wonton.ui.design.WontonSheet
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme

/**
 * 加联系人：选对方是 Bot 还是人（核心不去对方那里查，ADR-0050），填完整的 Matrix ID。[onSubmit] 的第二个参数为真时是人。
 */
@Composable
internal fun AddContactSheet(onDismiss: () -> Unit, onSubmit: (String, Boolean) -> Unit) {
    var userId by rememberSaveable { mutableStateOf("") }
    var person by rememberSaveable { mutableStateOf(false) }
    WontonSheet(onDismiss, stringResource(R.string.contacts_add)) { close ->
        val submit: () -> Unit = {
            val id = userId.trim()
            if (id.isNotEmpty()) close { onSubmit(id, person) }
        }
        Column(
            Modifier.fillMaxWidth().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                WontonRadioRow(stringResource(R.string.contacts_add_kind_bot), !person, true, { person = false })
                WontonRadioRow(stringResource(R.string.contacts_add_kind_person), person, true, { person = true })
            }
            WontonTextField(
                userId,
                { userId = it },
                Modifier.fillMaxWidth(),
                placeholder = stringResource(R.string.contacts_add_placeholder),
                label = stringResource(R.string.contacts_add),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email, imeAction = ImeAction.Send),
                keyboardActions = KeyboardActions(onSend = { submit() }),
            )
            WontonButton(
                stringResource(R.string.contacts_add_submit),
                submit,
                Modifier.fillMaxWidth(),
                enabled = userId.isNotBlank(),
            )
        }
    }
}

/** 删掉已生效的联系人前确认：两边都会停。 */
@Composable
internal fun RemoveContactDialog(contact: BotContact, onDismiss: () -> Unit, onConfirm: () -> Unit) {
    WontonDialog(
        stringResource(R.string.contacts_remove_title, contact.name()),
        stringResource(R.string.contacts_remove_message),
        onDismiss,
        listOf(
            WontonDialogAction(stringResource(R.string.common_cancel), onClick = onDismiss),
            WontonDialogAction(stringResource(R.string.common_remove), WontonButtonKind.DANGER, onConfirm),
        ),
    )
}
