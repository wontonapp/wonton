import Photos
import SwiftUI
import UniformTypeIdentifiers
import WontonClient

/// 回形针打开的附件面板（ADR-0109 第 9 节）：上面是相册里最近的照片，点一下选中、再点取消，能选几张看附件区还剩几格；
/// 底下浮着一排玻璃的切换，只列做得到的：相册、文件，整块到面板底、能滚。选了照片，那一排换成附言框和发送键，照片和附言作为一次发出去。
/// 相册权限到这里才问；没给就如实说，配去设置的按钮，系统的照片选择器不要权限，也留一个入口。
struct AttachSheet: View {
    /// 这次最多还能选几张。
    let limit: Int
    /// 旁聊里发不了文件（ADR-0093）。
    let allowsFiles: Bool
    /// 附言就是输入框的草稿：输入框里打了字再开面板，字跟着照片一起发；不发收起面板，字还在输入框里。
    @Binding var caption: String
    let send: ([PHAsset]) -> Void
    let chooseFiles: () -> Void
    let openSystemPicker: () -> Void
    @State private var access = PHPhotoLibrary.authorizationStatus(for: .readWrite)
    @State private var assets: [PHAsset] = []
    @State private var selected: [String] = []
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Group {
            switch access {
            case .authorized, .limited:
                grid
            case .denied, .restricted:
                denied
            default:
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        // 底下那一条浮在网格上，照片从它下面滚过去。
        .safeAreaInset(edge: .bottom, spacing: 0) { bottomBar }
        .background(Color(colors.canvas).ignoresSafeArea())
        .animation(.easeOut(duration: WontonTheme.motion.state), value: selected.isEmpty)
        .task {
            if access == .notDetermined {
                access = await PHPhotoLibrary.requestAuthorization(for: .readWrite)
            }
            if access == .authorized || access == .limited { assets = PhotoLibrary.recent() }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("attachSheet")
    }

    private var grid: some View {
        ScrollView {
            LazyVGrid(
                columns: Array(repeating: GridItem(.flexible(), spacing: Self.gap), count: Self.columns),
                spacing: Self.gap
            ) {
                ForEach(Array(assets.enumerated()), id: \.element.localIdentifier) { index, asset in
                    let order = selected.firstIndex(of: asset.localIdentifier)
                    Button {
                        toggle(asset.localIdentifier)
                    } label: {
                        PhotoCell(asset: asset, order: order.map { $0 + 1 })
                    }
                    .buttonStyle(.plain)
                    // 选满了，没选中的不能再点。
                    .disabled(order == nil && selected.count >= limit)
                    .accessibilityIdentifier("attachPhoto.\(index)")
                    .accessibilityAddTraits(order == nil ? [] : .isSelected)
                }
            }
            .padding(.top, WontonTheme.spacing.s24)
        }
    }

    private var denied: some View {
        VStack(spacing: WontonTheme.spacing.s16) {
            Text(.Composer.composerPhotoAccessDenied)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
                .multilineTextAlignment(.center)
            Button(.Common.commonOpenSettings) {
                if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) }
            }
            .buttonStyle(WontonButtonStyle(kind: .primary))
            Button(String(localized: .Composer.composerPhoto), action: openSystemPicker)
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                .accessibilityIdentifier("attachSystemPicker")
        }
        .padding(WontonTheme.spacing.pageInset)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityIdentifier("attachDenied")
    }

    /// 没选照片时是那一排切换，选了换成附言框和发送键。
    @ViewBuilder private var bottomBar: some View {
        if selected.isEmpty {
            tabs.transition(.opacity)
        } else {
            CaptionSendBar(
                placeholder: String(localized: .Photo.photoCaption), text: $caption, count: selected.count,
                cancel: { selected = [] },
                // 选了照片就能发；只打了附言没选照片时这一条不出现，写了照片或字任一个就能发。
                canSend: !caption.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !selected.isEmpty,
                send: { send(selected.compactMap { id in assets.first { $0.localIdentifier == id } }) },
                id: "attach"
            )
            .transition(.opacity)
        }
    }

    /// 一块玻璃胶囊，里面每项图标在上、短字在下，当前一项垫一层淡淡的字色。
    private var tabs: some View {
        HStack(spacing: 0) {
            tab(.imageFill, String(localized: .Composer.composerPhoto), isCurrent: true) {}
                .accessibilityIdentifier("attachPhotos")
            if allowsFiles {
                tab(.file, String(localized: .Composer.composerFile), isCurrent: false, action: chooseFiles)
                    .accessibilityIdentifier("attachFiles")
            }
        }
        .padding(WontonTheme.spacing.s4)
        .chatGlass(in: Capsule())
        .padding(.bottom, WontonTheme.spacing.s8)
    }

    private func tab(_ icon: WontonIcon, _ title: String, isCurrent: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            VStack(spacing: WontonTheme.spacing.s4) {
                icon.view()
                Text(title).wontonText(WontonTheme.typography.caption).fontWeight(.medium).lineLimit(1)
            }
            .foregroundStyle(Color(colors.textPrimary))
            .frame(width: WontonTheme.size.attachTab, height: WontonTheme.size.attachTab)
            .background {
                if isCurrent {
                    Capsule().fill(Color(colors.textPrimary).opacity(WontonTheme.ratio.tabHighlight))
                }
            }
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isCurrent ? .isSelected : [])
    }

    private func toggle(_ id: String) {
        if let index = selected.firstIndex(of: id) {
            selected.remove(at: index)
        } else if selected.count < limit {
            selected.append(id)
        }
    }

    private static let columns = 3
    private static let gap: CGFloat = 2
}

/// 网格里的一张照片：方的缩略图，选中的右上角写第几张。
private struct PhotoCell: View {
    let asset: PHAsset
    let order: Int?
    @State private var image: UIImage?
    @Environment(\.wontonColors) private var colors
    @Environment(\.displayScale) private var scale

    var body: some View {
        Color(colors.surfaceSubtle)
            .aspectRatio(1, contentMode: .fit)
            .overlay {
                if let image { Image(uiImage: image).resizable().scaledToFill() }
            }
            .clipped()
            .overlay(alignment: .topTrailing) { badge.padding(WontonTheme.spacing.s8) }
            .contentShape(Rectangle())
            .task(id: asset.localIdentifier) {
                let side = WontonTheme.size.messageImage * scale / 2
                image = await PhotoLibrary.thumbnail(asset, side: side)
            }
    }

    private var badge: some View {
        ZStack {
            Circle().fill(order == nil ? Color.black.opacity(WontonTheme.ratio.scrim / 2) : Color(colors.primary))
            Circle().strokeBorder(.white, lineWidth: 1.5)
            if let order {
                Text(verbatim: "\(order)")
                    .wontonText(WontonTheme.typography.caption)
                    .fontWeight(.semibold)
                    .foregroundStyle(Color(colors.onPrimary))
            }
        }
        .frame(width: WontonTheme.size.icon, height: WontonTheme.size.icon)
    }
}

/// 读相册：最近的照片、缩略图、原图的字节。
enum PhotoLibrary {
    /// 网格里最多列这么多张，再早的用系统选择器找。
    private static let recentLimit = 300

    static func recent() -> [PHAsset] {
        let options = PHFetchOptions()
        options.sortDescriptors = [NSSortDescriptor(key: "creationDate", ascending: false)]
        options.fetchLimit = recentLimit
        let result = PHAsset.fetchAssets(with: .image, options: options)
        return (0..<result.count).map { result.object(at: $0) }
    }

    static func thumbnail(_ asset: PHAsset, side: CGFloat) async -> UIImage? {
        let options = PHImageRequestOptions()
        options.deliveryMode = .highQualityFormat
        options.resizeMode = .fast
        options.isNetworkAccessAllowed = true
        return await withCheckedContinuation { continuation in
            // highQualityFormat 只回调一次，续体不会被恢复两回。
            PHImageManager.default().requestImage(
                for: asset, targetSize: CGSize(width: side, height: side), contentMode: .aspectFill, options: options
            ) { image, _ in
                continuation.resume(returning: image)
            }
        }
    }

    /// 原图的字节和类型；取不到（比如 iCloud 上的原图下不来）为空。
    /// `WONTON_UI_PHOTO_DELAY_MS`：界面测试把取字节拖慢，好看到等待中的样子。
    static func data(_ asset: PHAsset) async -> (Data, UTType?)? {
        if let delay = ProcessInfo.processInfo.environment["WONTON_UI_PHOTO_DELAY_MS"].flatMap(Int.init) {
            try? await Task.sleep(for: .milliseconds(delay))
        }
        let options = PHImageRequestOptions()
        options.deliveryMode = .highQualityFormat
        options.isNetworkAccessAllowed = true
        return await withCheckedContinuation { continuation in
            PHImageManager.default().requestImageDataAndOrientation(for: asset, options: options) { data, uti, _, _ in
                continuation.resume(returning: data.map { ($0, uti.flatMap { UTType($0) }) })
            }
        }
    }
}
