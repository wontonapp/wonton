package app.wonton.ui

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** 行尾给不给“更新”：上传的包看来源，预装插件看核心查到的报价；两样都没有时核心升不了。 */
class StoreRowUpdateTest {
    @Test
    fun preinstalledPluginWithAnOfferStillShowsUpdate() {
        assertTrue(storeRowShowsUpdate("1.0.0", "2.0.0", hasSource = false, hasOffer = true))
    }

    @Test
    fun uploadedPluginWithASourceShowsUpdate() {
        assertTrue(storeRowShowsUpdate("1.0.0", "2.0.0", hasSource = true, hasOffer = false))
    }

    @Test
    fun preinstalledPluginWithoutAnOfferGetsNoUpdate() {
        assertFalse(storeRowShowsUpdate("1.0.0", "2.0.0", hasSource = false, hasOffer = false))
    }

    @Test
    fun sameVersionGetsNoUpdate() {
        assertFalse(storeRowShowsUpdate("2.0.0", "2.0.0", hasSource = true, hasOffer = true))
    }
}
