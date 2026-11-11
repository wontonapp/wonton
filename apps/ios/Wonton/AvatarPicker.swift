import SwiftUI
import PhotosUI
import UniformTypeIdentifiers
import WontonClient

/// 相册选取（ADR-0044）。用系统的照片选择器（`PhotosPicker`，底下就是 PHPicker），免相册权限；
/// 图片拿原始字节（动图要原样交、PNG 要留透明），视频拷成本机临时文件再转码。
struct PickedMovie: Transferable {
    let url: URL

    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(contentType: .movie) { movie in
            SentTransferredFile(movie.url)
        } importing: { received in
            let copy = FileManager.default.temporaryDirectory
                .appendingPathComponent("wonton-avatar-\(UUID().uuidString).mov")
            try FileManager.default.copyItem(at: received.file, to: copy)
            return Self(url: copy)
        }
    }
}

extension PhotosPickerItem {
    /// 状态头像选了视频。
    var isMovie: Bool {
        supportedContentTypes.contains { $0.conforms(to: .movie) || $0.conforms(to: .mpeg4Movie) }
    }

    var isGIF: Bool {
        supportedContentTypes.contains(.gif)
    }
}

/// 选取的归类：转码和出错提示都按它走。
enum AvatarSlot: Equatable {
    /// 静态头像：只收图片；动图不合规时按具体的检查结果报。
    case staticAvatar
    /// 状态头像：图片或视频；动图不合规提示用视频（ADR-0044）。
    case state(BotActivity)

    var allowsVideo: Bool {
        if case .state = self { return true }
        return false
    }

    var state: BotActivity? {
        if case let .state(state) = self { return state }
        return nil
    }

    var filter: PHPickerFilter {
        allowsVideo ? .any(of: [.images, .videos]) : .images
    }
}
