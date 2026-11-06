package app.wonton.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.StoreUnreadable
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon

/** 来源里有、但清单读不懂的插件，写明来源和原因。[key] 只用于列表项的 key。 */
internal fun LazyListScope.unreadable(key: String, plugins: List<StoreUnreadable>) {
    items(plugins, key = { "unreadable:$key:${it.source}" }) { plugin ->
        Box(Modifier.padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s8)) {
            WontonNotice(WontonIcon.Warning, stringResource(R.string.store_unreadable, plugin.source, plugin.reason))
        }
    }
}
