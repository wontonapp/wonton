package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import app.wonton.protocol.OwnerProfile
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

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
        OwnerProfileFields(name, onName, email, onEmail)
        problem?.let { ProblemNotice(it) }
        WontonGroup { WontonRow("用户名", profile.username) }
    }
}

@Composable
internal fun OwnerProfileFields(
    name: String,
    onName: (String) -> Unit,
    email: String,
    onEmail: (String) -> Unit,
    nameProblem: String? = null,
    emailProblem: String? = null,
) {
    DetailSection("名字") {
        WontonTextField(
            name, onName, Modifier.fillMaxWidth(), placeholder = "必填", label = "名字",
            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Words),
        )
        nameProblem?.let { ProfileFieldProblem(it) }
    }
    DetailSection("邮箱") {
        WontonTextField(
            email, onEmail, Modifier.fillMaxWidth(), placeholder = "选填", label = "邮箱",
            keyboardOptions = KeyboardOptions(
                capitalization = KeyboardCapitalization.None,
                autoCorrectEnabled = false,
                keyboardType = KeyboardType.Email,
            ),
        )
        emailProblem?.let { ProfileFieldProblem(it) }
    }
}

@Composable
private fun ProfileFieldProblem(message: String) {
    Text(message, style = WontonTheme.typography.supporting, color = WontonTheme.colors.danger.toColor())
}
