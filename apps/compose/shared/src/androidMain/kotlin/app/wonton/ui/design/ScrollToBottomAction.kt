package app.wonton.ui.design

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.material3.Badge
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import app.wonton.ui.R
import app.wonton.ui.icons.Down
import app.wonton.ui.icons.WontonIcon

/**
 * 停在别处时的“回到底部”圆钮：数字是离开底部后到的消息数，没有就不写数字（ADR-0071）。
 * 命中区按 48dp 的点按门槛，取它和 FAB 尺寸里大的那个，requiredSize 让外层挤小时也不缩。
 */
@Composable
fun ScrollToBottomAction(count: Int, onClick: () -> Unit, modifier: Modifier = Modifier) {
    // 圆钮可以是 56 的 FAB，但命中区永远不小于 48 的点按门槛。
    val size = WontonTheme.size.floatingActionButton.coerceAtLeast(WontonTheme.size.target)
    val label = if (count > 0) {
        stringResource(R.string.room_scroll_to_bottom_count, count)
    } else {
        stringResource(R.string.room_scroll_to_bottom)
    }
    FloatingActionButton(
        onClick = onClick,
        modifier = modifier
            .requiredSize(size)
            .semantics { contentDescription = label },
        containerColor = WontonTheme.colors.primary.toColor(),
        contentColor = WontonTheme.colors.onPrimary.toColor(),
    ) {
        // 角标叠在圆钮外面：FAB 的 Surface 会按圆形裁剪内容，放进 BadgedBox 超出的部分会被切掉。
        Box {
            // 装饰：圆钮外层已经带了名字。
            Icon(WontonIcon.Down, contentDescription = null)
            if (count > 0) {
                Badge(
                    modifier = Modifier
                        .align(Alignment.TopEnd)
                        .offset(x = 6.dp, y = (-6).dp),
                ) {
                    Text(if (count > BADGE_COUNT_MAX) "$BADGE_COUNT_MAX+" else count.toString())
                }
            }
        }
    }
}

/** 角标数字超过这个数就写“99+”。 */
private const val BADGE_COUNT_MAX = 99
