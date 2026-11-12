package app.wonton.android

import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * 插件网页是 http://（经本地代理进隧道）。清单声明了 networkSecurityConfig，系统就不看清单里的
 * usesCleartextTraffic，只认这份配置：没写 cleartextTrafficPermitted，WebView 对插件页一律报
 * ERR_CLEARTEXT_NOT_PERMITTED（#302 到 #377 之间的 main 就是这样）。release 和 debug 各有一份，都要放开。
 */
class NetworkSecurityConfigTest {
    @Test
    fun bothVariantsLetPluginPagesUseHttp() {
        for (variant in listOf("main", "debug")) {
            val file = File("src/$variant/res/xml/network_security_config.xml")
            val base = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(file)
                .getElementsByTagName("base-config").item(0)
            val permitted = base.attributes.getNamedItem("cleartextTrafficPermitted")?.nodeValue
            assertEquals("$variant base-config must permit cleartext", "true", permitted)
        }
    }
}
