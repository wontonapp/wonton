package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import app.wonton.protocol.OwnerProfile
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme

@Composable
internal fun OwnerProfileForm(
    profile: OwnerProfile,
    name: String,
    onName: (String) -> Unit,
    email: String,
    onEmail: (String) -> Unit,
    problem: Problem?,
    modifier: Modifier = Modifier,
) {
    Column(modifier, verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap)) {
        val nameLabel = "名字"
        DetailSection(nameLabel) {
            WontonTextField(
                name,
                onName,
                Modifier.fillMaxWidth(),
                label = nameLabel,
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Words),
            )
        }
        val emailLabel = "邮箱"
        DetailSection(emailLabel) {
            WontonTextField(
                email,
                onEmail,
                Modifier.fillMaxWidth(),
                placeholder = "me@example.com",
                label = emailLabel,
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.None,
                    autoCorrectEnabled = false,
                    keyboardType = KeyboardType.Email,
                ),
            )
        }
        problem?.let { ProblemNotice(it) }
        WontonGroup { WontonRow("用户名", profile.username) }
    }
}
