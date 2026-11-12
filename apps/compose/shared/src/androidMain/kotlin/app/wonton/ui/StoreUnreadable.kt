package app.wonton.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.ui.Modifier
import app.wonton.protocol.StoreUnreadable
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

/** 来源里有、但清单读不懂的插件，写明来源和原因。 */
internal fun LazyListScope.unreadable(section: String, plugins: List<StoreUnreadable>) {
    items(plugins, key = { "unreadable:$section:${it.source}" }) { plugin ->
        Box(Modifier.padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s8)) {
            WontonNotice(WontonIcon.Warning, "${plugin.source} 读不懂：${plugin.reason}")
        }
    }
}
