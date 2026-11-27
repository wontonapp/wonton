// 引导选项行（#672）的四种状态：未选中、中性选中、装不了、已装但 install blocked 被抑制。
// 「已装」由调用点用 trailing 的“已安装”和 blocked=null 表达，StepOptionRow 本身不认识安装状态。
package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import app.wonton.ui.design.StepOptionRow
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

@Preview(name = "引导选项", locale = "en", widthDp = 390, heightDp = 480)
@Composable
internal fun StepOptionRowStatesPreview() {
    PreviewFrame {
        Column(
            Modifier.fillMaxWidth().padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            // 未选中。
            StepOptionRow(title = "Photos", selected = false, onSelect = {}, multiple = true)
            // 中性选中：只靠勾选标志，不铺强调色。
            StepOptionRow(
                title = "Files", selected = true, onSelect = {}, multiple = true,
                neutralSelection = true,
            )
            // 没装且装不了：红字原因，行禁用。
            StepOptionRow(
                title = "Camera", selected = false, onSelect = {}, multiple = true,
                blocked = "Needs a newer server",
            )
            // 已装但 install blocked 被抑制：同中性选中，尾部“已安装”。
            StepOptionRow(
                title = "Chat", selected = true, onSelect = {}, multiple = true,
                neutralSelection = true,
                trailing = {
                    Text(
                        "Installed",
                        Modifier.padding(WontonTheme.spacing.s12),
                        style = WontonTheme.typography.supporting,
                        color = WontonTheme.colors.textSecondary.toColor(),
                    )
                },
            )
        }
    }
}
