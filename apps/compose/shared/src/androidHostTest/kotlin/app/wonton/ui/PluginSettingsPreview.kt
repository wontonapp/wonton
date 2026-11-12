// 插件设置页的假数据和预览：按接口字段直接构造，标题说明按核心给的写法，壳里不拼。
// 参考图见 PreviewSnapshotsTest，录制命令见 .github/workflows/README.md。
@file:Suppress("MagicNumber")

package app.wonton.ui

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import app.wonton.protocol.PluginSettingField
import app.wonton.protocol.PluginSettingKind
import app.wonton.protocol.PluginSettingOption
import app.wonton.protocol.PluginSettingsView
import app.wonton.protocol.SettingValue
import app.wonton.protocol.SettingsFieldProblem
import app.wonton.protocol.SettingsFieldProblemKind
import app.wonton.protocol.SettingsForm
import app.wonton.ui.design.WontonTheme

/** 一项假设置：只填看得见的部分，其余取空值。 */
private fun field(
    name: String,
    title: String,
    kind: PluginSettingKind,
    required: Boolean = false,
    value: SettingValue? = null,
    default: SettingValue? = null,
    problem: SettingsFieldProblem? = null,
    services: List<String> = emptyList(),
) = PluginSettingField(
    name = name,
    title = title,
    description = null,
    required = required,
    kind = kind,
    value = value,
    default = default,
    services = services,
    problem = problem,
)

private fun option(value: String, title: String) =
    PluginSettingOption(value = SettingValue.Text(value), title = title)

/** 每种控件一项：开关、整数、小数、文本、单选、多选，和一项这个 App 还画不出来的。 */
internal fun previewSettingsFields() = PluginSettingsView(
    schemaDigest = "sha256:preview",
    form = SettingsForm(
        listOf(
            field(
                "eula", "Accept the Minecraft EULA", PluginSettingKind.Toggle,
                value = SettingValue.Flag(false), services = listOf("server"),
            ),
            field(
                "max_players", "Max players", PluginSettingKind.Integer(min = 1L, max = 100L),
                value = SettingValue.Integer(20),
            ),
            field(
                "backup_interval", "Backup interval (hours)", PluginSettingKind.Decimal(min = "0.5", max = "24.0"),
                value = SettingValue.Decimal("1.5"),
            ),
            field(
                "motd", "Message of the day",
                PluginSettingKind.Text(minLength = 1u, maxLength = 40u, pattern = null),
                value = SettingValue.Text("Play together"),
            ),
            field(
                "difficulty", "Difficulty",
                PluginSettingKind.Choice(
                    listOf(option("peaceful", "Peaceful"), option("normal", "Normal")),
                    multiple = false,
                ),
                value = SettingValue.Text("normal"),
            ),
            field(
                "modules", "Enabled modules",
                PluginSettingKind.Choice(
                    listOf(
                        option("chat", "Chat"),
                        option("map", "Map"),
                        option("economy", "Economy"),
                    ),
                    multiple = true,
                ),
                value = SettingValue.Texts(listOf("chat")),
            ),
            field("new_kind", "A setting this app can't render yet", PluginSettingKind.Other),
        ),
    ),
    changedAtMs = null,
)

/** 有问题的一页：一项标红（核心给的话）、一项必填没填，外加画不出来的那项。 */
internal fun previewSettingsProblems() = PluginSettingsView(
    schemaDigest = "sha256:preview",
    form = SettingsForm(
        listOf(
            field(
                "max_players", "Max players", PluginSettingKind.Integer(min = 1L, max = 100L),
                value = SettingValue.Integer(500),
                problem = SettingsFieldProblem(
                    SettingsFieldProblemKind.ABOVE_MAXIMUM,
                    "Must be a number between 1 and 100",
                ),
            ),
            field(
                "motd", "Message of the day",
                PluginSettingKind.Text(minLength = 1u, maxLength = 40u, pattern = null),
                required = true,
            ),
            field("new_kind", "A setting this app can't render yet", PluginSettingKind.Other),
        ),
    ),
    changedAtMs = null,
)

/** 每种控件一项，各画各的。 */
@Preview(name = "插件设置项", locale = "en", widthDp = 390, heightDp = 760)
@Composable
internal fun PluginSettingsFieldsPreview() {
    PreviewFrame {
        SettingsForm(
            previewSettingsFields(), emptyMap(), emptyMap(), null, restartHint = false,
            fieldProblems = emptyMap(),
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState())
                .padding(WontonTheme.spacing.pageInset),
            onEdit = { _, _ -> },
            onDraft = { _, _ -> },
        )
    }
}

/** 有问题：控件旁标红、必填缺值提示、要更新 App 的那项。 */
@Preview(name = "插件设置问题", locale = "en", widthDp = 390, heightDp = 420)
@Composable
internal fun PluginSettingsProblemsPreview() {
    PreviewFrame {
        SettingsForm(
            previewSettingsProblems(), emptyMap(), emptyMap(), null, restartHint = false,
            fieldProblems = emptyMap(),
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState())
                .padding(WontonTheme.spacing.pageInset),
            onEdit = { _, _ -> },
            onDraft = { _, _ -> },
        )
    }
}

/** 保存成功后的提示：改动的项有服务用到，应用会重启一下。 */
@Preview(name = "插件设置重启提示", locale = "en", widthDp = 390, heightDp = 200)
@Composable
internal fun PluginSettingsRestartHintPreview() {
    PreviewFrame {
        SettingsForm(
            previewSettingsFields(), emptyMap(), emptyMap(), null, restartHint = true,
            fieldProblems = emptyMap(),
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState())
                .padding(WontonTheme.spacing.pageInset),
            onEdit = { _, _ -> },
            onDraft = { _, _ -> },
        )
    }
}

/** 保存进行中：按钮换成转圈。 */
@Preview(name = "插件设置保存中", locale = "en", widthDp = 390, heightDp = 120)
@Composable
internal fun PluginSettingsSavingPreview() {
    PreviewFrame {
        SettingsSaveArea(saving = true, blocked = false, onSave = {})
    }
}
