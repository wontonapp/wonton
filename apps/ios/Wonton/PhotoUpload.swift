import Foundation
import ImageIO
import UniformTypeIdentifiers
import WontonClient

/// 尺寸上限、转码和缩略图的规则见 ADR-0005 的“App 内聊天的界面”。
enum PhotoUpload {
    static let maxSide = 4096
    static let thumbnailSize = 800

    enum Failure: Error, CustomStringConvertible {
        case unreadable
        case unencodable

        var description: String {
            switch self {
            case .unreadable: return "读不出这张照片"
            case .unencodable: return "照片转码失败"
            }
        }
    }

    static func make(data: Data, type: UTType?) throws(Failure) -> ImageUpload {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let size = pixelSize(source) else { throw .unreadable }
        let longerSide = max(size.width, size.height)
        if type?.conforms(to: .gif) == true {
            // 原样发，保留动画。
            return ImageUpload(filename: "photo.gif", mimeType: "image/gif", data: data,
                               width: UInt32(size.width), height: UInt32(size.height),
                               thumbnail: try thumbnail(source, longerSide: longerSide))
        }
        guard let image = upright(source, maxPixelSize: mainSide(longerSide)) else { throw .unreadable }
        let width = UInt32(image.width)
        let height = UInt32(image.height)
        let thumbnail = try thumbnail(source, longerSide: max(image.width, image.height))
        if type?.conforms(to: .png) == true {
            guard let png = encode(image, as: .png, options: [:]) else { throw .unencodable }
            return ImageUpload(filename: "photo.png", mimeType: "image/png", data: png,
                               width: width, height: height, thumbnail: thumbnail)
        }
        let quality = [kCGImageDestinationLossyCompressionQuality: 0.9]
        guard let jpeg = encode(image, as: .jpeg, options: quality) else { throw .unencodable }
        return ImageUpload(filename: "photo.jpg", mimeType: "image/jpeg", data: jpeg,
                           width: width, height: height, thumbnail: thumbnail)
    }

    /// 文件头里的像素尺寸，不解码。
    private static func pixelSize(_ source: CGImageSource) -> (width: Int, height: Int)? {
        guard let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int else { return nil }
        return (width, height)
    }

    /// 主图的长边：超过 `maxSide` 时取原图的 1/2、1/4…，取到不超过 `maxSide` 为止。JPEG 的解码器只在这些比例上
    /// 边解边缩，缩到别的尺寸时 ImageIO 先解出原尺寸（macOS 27.2 实测：8064×6048 的 JPEG 缩到 4096 峰值 492 MB，
    /// 缩到 4032 是 211 MB）。
    private static func mainSide(_ longerSide: Int) -> Int {
        var side = longerSide
        while side > maxSide { side = (side + 1) / 2 }
        return side
    }

    private static func thumbnail(_ source: CGImageSource, longerSide: Int) throws(Failure) -> ImageThumbnail? {
        guard longerSide > thumbnailSize else { return nil }
        guard let small = upright(source, maxPixelSize: thumbnailSize) else { throw .unreadable }
        let quality = [kCGImageDestinationLossyCompressionQuality: 0.8]
        guard let data = encode(small, as: .jpeg, options: quality) else { throw .unencodable }
        return ImageThumbnail(mimeType: "image/jpeg", data: data,
                              width: UInt32(small.width), height: UInt32(small.height))
    }

    /// `kCGImageSourceCreateThumbnailWithTransform` 按 EXIF 方向转正；比 `maxPixelSize` 小的图不放大。
    private static func upright(_ source: CGImageSource, maxPixelSize: Int) -> CGImage? {
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixelSize
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }

    /// 只写像素，不传图片属性：原图的 EXIF、GPS、XMP、IPTC 都不会写进去。ImageIO 自己会写一段 EXIF，只有色彩空间和
    /// 像素尺寸。
    private static func encode(_ image: CGImage, as type: UTType, options: [CFString: Any]) -> Data? {
        let data = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(data, type.identifier as CFString, 1, nil) else {
            return nil
        }
        CGImageDestinationAddImage(destination, image, options as CFDictionary)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return data as Data
    }
}
