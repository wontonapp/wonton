import CryptoKit
import SwiftUI
import UIKit
import WontonClient

/// 海报、剧照、背景图的加载：字节走客户端核的本机回环。回环的端口和令牌每次启动都换，缓存键去掉它们
/// （地址里第一段路径之后的部分），App 重启后不用重取；换了服务器或账号同一路径是另一张图，键前面带上
/// 客户端核给的作用域。内存里留解码好的图（按解码后的字节算上限），磁盘上留原字节，超了上限按最久没用的删。
actor VideoImages {
    static let shared = VideoImages()

    private let memory = NSCache<NSString, UIImage>()
    private let directory: URL
    private let diskLimit = 300 * 1024 * 1024
    private var loading: [String: Task<UIImage?, Never>] = [:]
    private var writes = 0

    init() {
        let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        directory = caches.appending(path: "video-artwork", directoryHint: .isDirectory)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        // 海报墙一屏几十张，每张解码后几 MB：只数张数挡不住内存。
        memory.totalCostLimit = 120 * 1024 * 1024
    }

    /// 回环地址 `http://127.0.0.1:<端口>/<令牌>/items/…` 在 `scope` 下的缓存键：作用域加 `/items/…` 连同查询串。
    nonisolated static func key(_ url: URL, scope: String) -> String {
        let parts = url.path(percentEncoded: true).split(separator: "/", maxSplits: 1, omittingEmptySubsequences: true)
        let rest = parts.count > 1 ? String(parts[1]) : url.path(percentEncoded: true)
        let path = url.query(percentEncoded: true).map { "\(rest)?\($0)" } ?? rest
        return "\(scope)\n\(path)"
    }

    func image(_ url: URL, scope: String) async -> UIImage? {
        let key = Self.key(url, scope: scope)
        if let hit = memory.object(forKey: key as NSString) { return hit }
        if let running = loading[key] { return await running.value }
        let task = Task { await fetch(url, key: key) }
        loading[key] = task
        let image = await task.value
        loading[key] = nil
        return image
    }

    private func file(_ key: String) -> URL {
        let digest = SHA256.hash(data: Data(key.utf8)).map { String(format: "%02x", $0) }.joined()
        return directory.appending(path: digest)
    }

    private func fetch(_ url: URL, key: String) async -> UIImage? {
        let file = file(key)
        if let data = try? Data(contentsOf: file), let image = UIImage(data: data) {
            try? FileManager.default.setAttributes([.modificationDate: Date()], ofItemAtPath: file.path)
            remember(image, key)
            return image
        }
        // 取不到（没有这张、断了）就不显示图，卡片留占位：图是装饰，不单独报错。
        guard let (data, response) = try? await URLSession.shared.data(from: url),
              (response as? HTTPURLResponse)?.statusCode == 200,
              let image = UIImage(data: data) else { return nil }
        remember(image, key)
        try? data.write(to: file, options: .atomic)
        writes += 1
        if writes % 50 == 0 { trim() }
        return image
    }

    /// 按解码后的字节记进内存。
    private func remember(_ image: UIImage, _ key: String) {
        let cost = image.cgImage.map { $0.bytesPerRow * $0.height }
            ?? Int(image.size.width * image.scale * image.size.height * image.scale * 4)
        memory.setObject(image, forKey: key as NSString, cost: cost)
    }

    /// 超了上限按修改时间（取用时会刷新）从旧往新删，删到上限的八成。
    private func trim() {
        let keys: [URLResourceKey] = [.contentModificationDateKey, .fileSizeKey]
        guard let files = try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: keys)
        else { return }
        let entries = files.compactMap { url -> CachedFile? in
            guard let values = try? url.resourceValues(forKeys: Set(keys)) else { return nil }
            let used = values.contentModificationDate ?? .distantPast
            return CachedFile(url: url, used: used, size: values.fileSize ?? 0)
        }
        var total = entries.reduce(0) { $0 + $1.size }
        guard total > diskLimit else { return }
        for file in entries.sorted(by: { $0.used < $1.used }) where total > diskLimit * 8 / 10 {
            try? FileManager.default.removeItem(at: file.url)
            total -= file.size
        }
    }
}

private struct CachedFile {
    let url: URL
    let used: Date
    let size: Int
}

/// 一张图：没有地址或取不到时是占位色块，上面可以放片名。图铺满给它的框、裁掉多出的部分；
/// 台标这类要整张露出来的传 `fit`，没有底色。
struct VideoArtwork<Placeholder: View>: View {
    let url: String?
    var fit = false
    @ViewBuilder var placeholder: Placeholder
    @State private var image: UIImage?
    @Environment(\.wontonColors) private var colors
    @Environment(\.videoCacheScope) private var scope

    var body: some View {
        // 底下是可伸缩的色块，图放在 overlay 里：图的原始尺寸不能把框撑大。
        (fit ? Color.clear : Color(colors.surfaceSubtle))
            .overlay {
                if let image {
                    Image(uiImage: image)
                        .resizable()
                        .aspectRatio(contentMode: fit ? .fit : .fill)
                        .transition(.opacity)
                } else {
                    placeholder
                }
            }
            .clipped()
            .task(id: "\(scope)\n\(url ?? "")") {
                guard let url, let address = URL(string: url) else {
                    image = nil
                    return
                }
                let loaded = await VideoImages.shared.image(address, scope: scope)
                withAnimation(.easeOut(duration: 0.2)) { image = loaded }
            }
    }
}

extension VideoArtwork where Placeholder == EmptyView {
    init(url: String?) {
        self.init(url: url, fit: false) { EmptyView() }
    }
}

extension EnvironmentValues {
    /// 图片缓存的作用域（`video_cache_scope`），影视页的根设一次。
    @Entry var videoCacheScope = ""
}
