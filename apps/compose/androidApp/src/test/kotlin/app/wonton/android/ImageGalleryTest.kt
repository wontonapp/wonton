package app.wonton.android

import android.app.Application
import androidx.test.ext.junit.runners.AndroidJUnit4
import app.wonton.ui.galleryName
import app.wonton.ui.imageMime
import app.wonton.ui.sniffImageMime
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * 存相册和分享前对文件名和 MIME 的处理：MIME 按声明、文件头、扩展名的顺序取；文件名只留
 * 最后一个路径段和合法字符，没有扩展名时按 MIME 补上。
 */
@RunWith(AndroidJUnit4::class)
// 不起 WontonApplication：它的静态初始化要加载 Android 的 arm64 原生库，JVM 上装不进。
@Config(sdk = [36], application = Application::class)
class ImageGalleryTest {
    private val jpeg = byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte(), 0xE0.toByte())
    private val png = byteArrayOf(
        0x89.toByte(), 0x50.toByte(), 0x4E.toByte(), 0x47.toByte(), 0x0D, 0x0A, 0x1A, 0x0A,
    )

    @Test
    fun sniffRecognizesCommonImageHeaders() {
        assertEquals("image/jpeg", sniffImageMime(jpeg))
        assertEquals("image/png", sniffImageMime(png))
        assertEquals("image/gif", sniffImageMime("GIF89a".toByteArray()))
        assertEquals("image/bmp", sniffImageMime("BM".toByteArray()))
        val webp = "RIFF\u0000\u0000\u0000\u0000WEBP".toByteArray()
        assertEquals("image/webp", sniffImageMime(webp))
        assertEquals(null, sniffImageMime("not an image".toByteArray()))
        assertEquals(null, sniffImageMime(ByteArray(0)))
    }

    @Test
    fun declaredImageMimeWinsOverSniffedBytes() {
        assertEquals("image/png", imageMime("image/png", jpeg, "a.jpg"))
        // 声明不可信（不是 image/）时按文件头认。
        assertEquals("image/jpeg", imageMime("application/octet-stream", jpeg, "a.jpg"))
        assertEquals(null, imageMime(null, "garbage".toByteArray(), "a"))
        // 文件头认不出时按扩展名猜。
        assertEquals("image/jpeg", imageMime(null, "garbage".toByteArray(), "photo.JPG"))
    }

    @Test
    fun galleryNameKeepsOnlyTheLastPathSegmentAndLegalChars() {
        assertEquals("photo.jpg", galleryName("photo.jpg", null))
        assertEquals("name.png", galleryName("../evil/na:me.png", null))
        assertEquals("b.jpg", galleryName("a\\b.jpg", null))
        assertEquals("wonton-image.png", galleryName("", "image/png"))
        assertEquals("noext.png", galleryName("noext", "image/png"))
        // MIME 认不出时保持无扩展名。
        assertEquals("noext", galleryName("noext", null))
    }
}
