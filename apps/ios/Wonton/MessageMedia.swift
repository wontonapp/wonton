import SwiftUI
import WontonClient

/// 同一次发送的一组照片（发送者、组 id 和按组内顺序的照片）：查看器在同组里翻页，时间线变了按它重取。
struct PhotoGroup {
    let senderId: String
    let id: String
    var images: [ImageContent]
}

/// 组键：组 id 只有 32 位，和发送者一起才是 ADR-0094 说的那个组。
struct PhotoGroupKey: Hashable {
    let senderId: String
    let id: String
}

/// 消息里的图片：先画缩略图，点了看原图。
struct MediaImage: View {
    let image: ImageContent
    /// 这一条属于哪一组照片；不属于多张一组时是 nil，查看器就是单张。
    let group: PhotoGroup?
    let core: ClientCore
    let sendState: SendState
    @Environment(RoomPresenter.self) private var presenter
    @State private var preview: Result<UIImage, MediaFailure>?

    var body: some View {
        Group {
            switch preview {
            case let .success(loaded):
                Image(uiImage: loaded).resizable().scaledToFit()
            case let .failure(failure):
                Label {
                    Text(failure.message)
                } icon: {
                    WontonIcon.imageBroken.view(size: WontonTheme.size.iconSmall)
                }
                .wontonText(WontonTheme.typography.supporting)
            case nil:
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(maxWidth: WontonTheme.size.messageImage)
        .aspectRatio(ratio, contentMode: .fit)
        .clipShape(RoundedRectangle(cornerRadius: WontonTheme.radii.control))
        .overlay(alignment: .bottom) {
            if case let .sending(progress) = sendState {
                if let progress {
                    ProgressView(value: progress)
                        .progressViewStyle(.linear)
                        .frame(maxWidth: .infinity)
                        .padding(4)
                } else {
                    ProgressView()
                        .padding(4)
                }
            }
        }
        .contentShape(Rectangle())
        .onTapGesture { presenter.showOriginal(image, group: group) }
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(Text(.Message2.messagePhoto))
        .accessibilityIdentifier("messagePhoto.\(image.filename)")
        .task(id: image.thumbnail ?? image.media) {
            preview = await MediaFailure.load(image.thumbnail ?? image.media, from: core)
        }
    }

    private var ratio: CGFloat? {
        guard let width = image.width, let height = image.height, width > 0, height > 0 else { return nil }
        return CGFloat(width) / CGFloat(height)
    }
}

struct OriginalImage: View {
    let image: ImageContent
    let core: ClientCore
    @Environment(\.dismiss) private var dismiss
    @State private var original: Result<UIImage, MediaFailure>?

    var body: some View {
        NavigationStack {
            Group {
                switch original {
                case let .success(loaded):
                    Image(uiImage: loaded).resizable().scaledToFit()
                case let .failure(failure):
                    WontonNotice(
                        icon: .imageBroken, title: String(localized: .Message1.messageOriginalUnavailable),
                        lines: [failure.message]
                    )
                        .padding(WontonTheme.spacing.pageInset)
                case nil:
                    ProgressView()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(.black)
            .navigationTitle(image.filename)
            .navigationBarTitleDisplayMode(.inline)
            .toolbarColorScheme(.dark, for: .navigationBar)
            .toolbarBackground(.black, for: .navigationBar)
            .toolbarBackground(.visible, for: .navigationBar)
            .toolbar { Button(.Common.commonDone) { dismiss() } }
        }
        .task { original = await MediaFailure.load(image.media, from: core) }
    }
}

/// 媒体取不到的原因，引用缩略图和原图共用。
struct MediaFailure: Error {
    let message: String

    static func load(_ media: MediaHandle, from core: ClientCore) async -> Result<UIImage, MediaFailure> {
        do {
            let data = try await core.loadMedia(media: media)
            guard let image = UIImage(data: data) else {
                return .failure(MediaFailure(message: String(localized: .Message0.messageImageFormatUnsupported)))
            }
            return .success(image)
        } catch {
            return .failure(MediaFailure(message: error.problem.title))
        }
    }
}
