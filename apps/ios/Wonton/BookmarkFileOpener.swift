import Foundation
import WontonClient

/// 按书签打开要上传的文件。文件选择器给的地址 App 重启后就失效了，接着传要靠书签再打开一次。
final class BookmarkFileOpener: FileOpener {
    static func handle(for url: URL) throws -> String {
        let accessing = url.startAccessingSecurityScopedResource()
        defer { if accessing { url.stopAccessingSecurityScopedResource() } }
        return try url.bookmarkData().base64EncodedString()
    }

    func openSource(handle: String) throws -> Int32 {
        guard let bookmark = Data(base64Encoded: handle) else {
            throw FileOpenError.Unavailable(reason: "书签读不懂")
        }
        let url: URL
        do {
            var isStale = false
            url = try URL(resolvingBookmarkData: bookmark, bookmarkDataIsStale: &isStale)
        } catch {
            throw FileOpenError.Unavailable(reason: error.localizedDescription)
        }
        let accessing = url.startAccessingSecurityScopedResource()
        defer { if accessing { url.stopAccessingSecurityScopedResource() } }

        // 文件可能只在 iCloud 上：协调读取会先把它取下来。核心在后台线程上调这里，等待不卡界面。
        var coordination: NSError?
        var opened: Result<Int32, FileOpenError> = .failure(.Unavailable(reason: "系统没有让 App 读这个文件"))
        NSFileCoordinator().coordinate(readingItemAt: url, error: &coordination) { readable in
            opened = Self.open(readable)
        }
        if let coordination { throw FileOpenError.Unavailable(reason: coordination.localizedDescription) }
        return try opened.get()
    }

    private static func open(_ url: URL) -> Result<Int32, FileOpenError> {
        let descriptor = Darwin.open(url.path(percentEncoded: false), O_RDONLY)
        if descriptor >= 0 { return .success(descriptor) }
        let reason = String(cString: strerror(errno))
        return errno == EACCES || errno == EPERM
            ? .failure(.PermissionDenied(reason: reason))
            : .failure(.Unavailable(reason: reason))
    }
}
