package app.wonton.ui

import android.app.Application
import android.content.Context
import androidx.core.content.FileProvider
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * 下载目录按服务器分层在 files/servers/<n>/downloads/ 后，FileProvider 的根必须盖住它，
 * 盖不住时点开和分享都在 getUriForFile 上崩；盖不住的路径要返回提示，不许崩。
 *
 * 三个断言放同一个方法里：FileProvider 按 authority 静态缓存根路径，Robolectric 每个方法
 * 换新的私有目录，分开方法时先跑的会把缓存污染成旧环境，后面方法就匹配不上。
 */
@RunWith(AndroidJUnit4::class)
// 不起 WontonApplication：它的静态初始化要加载 Android 的 arm64 原生库，JVM 上装不进。
@Config(sdk = [36], application = Application::class)
class FileHandoffTest {
    private val context: Context = ApplicationProvider.getApplicationContext()

    private fun write(vararg segments: String): File {
        val file = File(context.filesDir, segments.joinToString("/"))
        file.parentFile!!.mkdirs()
        file.writeBytes(byteArrayOf(1))
        return file
    }

    @Test
    fun downloadRootsCoverPerServerDirectoriesAndOutsideRootsAreRefused() {
        val download = write("servers", "1", "downloads", "abc123", "账单.pdf")
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", download)
        assertEquals("content", uri.scheme)

        // files/ 根没放开，交给系统的失败要变成提示，不能崩。
        val outside = write("secret.txt")
        assertNotNull(openWithSystem(context, outside.absolutePath, "text/plain"))
        assertNotNull(shareWithSystem(context, outside.absolutePath, "text/plain"))
    }
}
