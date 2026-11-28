import ImageIO
import SwiftUI
import UIKit
import WontonClient

/// 要哪张图。`scope` 是服务器加相册插件，条目 id 只在这个范围里唯一。
struct GalleryImageRequest: Hashable, Sendable {
    enum Grade: String, Sendable {
        case thumbnail
        case preview
    }

    enum Subject: Hashable, Sendable {
        case item(id: String, grade: Grade)
        case person(id: String)
    }

    let scope: String
    let subject: Subject
    /// 解码后长边的像素上限：网格按格子大小解，不把整张预览图留在内存里。
    let maxPixel: Int

    /// 盘上缓存的键：服务器、相册插件、条目 id 加档位。不用回环地址当键，它每次启动换端口和令牌。
    var storageKey: String {
        switch subject {
        case let .item(id, grade): "\(scope)/item/\(id)/\(grade.rawValue)"
        case let .person(id): "\(scope)/person/\(id)"
        }
    }

    var memoryKey: String { "\(storageKey)@\(maxPixel)" }
}

/// 图没取到的原因，按核心给的类别出字。
struct GalleryImageFailure: Error, Sendable {
    let problem: Problem
}

/// 相册的图片加载：解好的图留在内存里，字节留在盘上。
///
/// 盘上用系统的 `URLCache`（有上限、按最近使用淘汰），不另带图片库；键写成一个不会真去访问的地址，
/// 由 `storageKey` 拼出。字节经客户端核的本机回环取，不经绑定层搬。
@MainActor
final class GalleryImagePipeline {
    static let shared = GalleryImagePipeline()

    private let memory = NSCache<NSString, UIImage>()
    private var loading: [String: Task<Result<UIImage, GalleryImageFailure>, Never>] = [:]

    private init() {
        memory.totalCostLimit = 128 << 20
    }

    /// 内存里已经有的：格子滚回来时不闪占位图。
    func cached(_ request: GalleryImageRequest) -> UIImage? {
        memory.object(forKey: request.memoryKey as NSString)
    }

    func image(_ request: GalleryImageRequest, core: ClientCore) async -> Result<UIImage, GalleryImageFailure> {
        let key = request.memoryKey
        if let hit = memory.object(forKey: key as NSString) { return .success(hit) }
        if let running = loading[key] { return await running.value }
        // 格子滚走了也让它取完：结果进缓存，滚回来就有。
        let task = Task.detached(priority: .userInitiated) { await GalleryImageFetcher.fetch(request, core: core) }
        loading[key] = task
        let result = await task.value
        loading[key] = nil
        if case let .success(image) = result {
            let cost = Int(image.size.width * image.scale * image.size.height * image.scale * 4)
            memory.setObject(image, forKey: key as NSString, cost: cost)
        }
        return result
    }
}

/// 取字节和解码都不在主线程上：盘上缓存、回环请求、按上限缩着解。
private enum GalleryImageFetcher {
    static let disk: URLCache = {
        let directory = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first?
            .appending(path: "gallery", directoryHint: .isDirectory)
        return URLCache(memoryCapacity: 0, diskCapacity: 768 << 20, directory: directory)
    }()

    static let session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.urlCache = nil
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.httpMaximumConnectionsPerHost = 6
        return URLSession(configuration: configuration)
    }()

    static func cacheRequest(_ key: String) -> URLRequest? {
        let path = key.split(separator: "/").compactMap {
            String($0).addingPercentEncoding(withAllowedCharacters: .alphanumerics)
        }
        return URL(string: "https://gallery-cache.invalid/" + path.joined(separator: "/")).map { URLRequest(url: $0) }
    }

    static func fetch(
        _ request: GalleryImageRequest, core: ClientCore
    ) async -> Result<UIImage, GalleryImageFailure> {
        let stored = cacheRequest(request.storageKey)
        if let stored, let hit = disk.cachedResponse(for: stored), let image = decode(hit.data, request.maxPixel) {
            return .success(image)
        }
        do {
            let address: String = switch request.subject {
            case let .item(id, grade):
                try await core.photoUrl(id: id, rendition: grade == .thumbnail ? .thumbnail : .preview)
            case let .person(id):
                try await core.photoPersonUrl(id: id)
            }
            guard let url = URL(string: address) else {
                return .failure(unreadable(URLError(.badURL).localizedDescription))
            }
            let (data, response) = try await session.data(from: url)
            if let http = response as? HTTPURLResponse, !(200 ..< 300).contains(http.statusCode) {
                let body = String(data: data.prefix(400), encoding: .utf8) ?? ""
                return .failure(unreadable("HTTP \(http.statusCode) \(body)"))
            }
            guard let image = decode(data, request.maxPixel) else {
                return .failure(unreadable(URLError(.cannotDecodeContentData).localizedDescription))
            }
            if let stored, let kept = HTTPURLResponse(
                url: stored.url ?? url, statusCode: 200, httpVersion: "HTTP/1.1",
                headerFields: ["Cache-Control": "max-age=31536000", "Content-Type": response.mimeType ?? "image/jpeg"]
            ) {
                disk.storeCachedResponse(CachedURLResponse(response: kept, data: data), for: stored)
            }
            return .success(image)
        } catch let MediaError.Failed(failure) {
            return .failure(GalleryImageFailure(problem: failure.galleryProblem))
        } catch {
            return .failure(unreadable(error.localizedDescription))
        }
    }

    static func unreadable(_ detail: String) -> GalleryImageFailure {
        GalleryImageFailure(problem: Problem(
            title: GalleryText.imageFailed, nextStep: GalleryText.problemRetryNext, detail: detail
        ))
    }

    /// 按上限缩着解，方向一并转正；解码放在调用方的后台任务里。
    static func decode(_ data: Data, _ maxPixel: Int) -> UIImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixel
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        return UIImage(cgImage: image)
    }
}

/// 一张相册里的图：先画 ThumbHash 占位，图到了淡入；取不到时留着占位、角上标一下。
struct GalleryImage: View {
    let request: GalleryImageRequest
    let thumbhash: String?
    let core: ClientCore
    var contentMode: ContentMode = .fill
    @State private var image: UIImage?
    @State private var failed = false
    @Environment(\.wontonColors) private var colors

    init(request: GalleryImageRequest, thumbhash: String?, core: ClientCore, contentMode: ContentMode = .fill) {
        self.request = request
        self.thumbhash = thumbhash
        self.core = core
        self.contentMode = contentMode
        _image = State(initialValue: GalleryImagePipeline.shared.cached(request))
    }

    var body: some View {
        // 占位和图各自铺在同一块底上再裁掉多出来的：放进一个 ZStack 里按各自的宽高比撑开，两层会对不齐。
        Color(colors.surfaceSubtle)
            .overlay {
                if let placeholder = thumbhash.flatMap(ThumbHash.image) {
                    Image(uiImage: placeholder).resizable().aspectRatio(contentMode: contentMode)
                }
            }
            .overlay {
                if let image {
                    Image(uiImage: image).resizable().aspectRatio(contentMode: contentMode)
                        .transition(.opacity)
                }
            }
            .clipped()
            .overlay {
                if failed {
                    WontonIcon.imageBroken.view(size: WontonTheme.size.iconSmall)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            }
            .task(id: request) {
                if let hit = GalleryImagePipeline.shared.cached(request) {
                    image = hit
                    failed = false
                    return
                }
                image = nil
                let result = await GalleryImagePipeline.shared.image(request, core: core)
                guard !Task.isCancelled else { return }
                switch result {
                case let .success(loaded):
                    failed = false
                    withAnimation(.easeOut(duration: WontonTheme.motion.state)) { image = loaded }
                case .failure:
                    failed = true
                }
            }
    }
}
