import Photos
import SwiftUI
import UniformTypeIdentifiers
import WontonClient

/// 回形针打开的附件面板（ADR-0109 第 8 节）：上面是相册里最近的照片，点一下选中、再点取消，能选几张看附件区还剩几格；
/// 底下一排切换，只列做得到的：照片、文件。选好的交给输入栏，照旧先进附件区（ADR-0094），旁聊里直接发（ADR-0093）。
/// 相册权限到这里才问；没给就如实说，配去设置的按钮，系统的照片选择器不要权限，也留一个入口。
struct AttachSheet: View {
    /// 这次最多还能选几张。
    let limit: Int
    /// 旁聊里发不了文件（ADR-0093）。
    let allowsFiles: Bool
    let pick: ([PHAsset]) -> Void
    let chooseFiles: () -> Void
    let openSystemPicker: () -> Void
    @State private var access = PHPhotoLibrary.authorizationStatus(for: .readWrite)
    @State private var assets: [PHAsset] = []
    @State private var selected: [String] = []
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: 0) {
            switch access {
            case .authorized, .limited:
                grid
            case .denied, .restricted:
                denied
            default:
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            bottomBar
        }
        .background(Color(colors.canvas).ignoresSafeArea())
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

    /// 选了照片时是“完成”，没选时是那一排切换。
    @ViewBuilder private var bottomBar: some View {
        if selected.isEmpty {
            HStack(spacing: 0) {
                tab(.image, String(localized: .Composer.composerPhoto), isCurrent: true) {}
                if allowsFiles {
                    tab(.attach, String(localized: .Composer.composerFile), isCurrent: false, action: chooseFiles)
                        .accessibilityIdentifier("attachFiles")
                }
            }
            .padding(.top, WontonTheme.spacing.s8)
        } else {
            Button {
                pick(selected.compactMap { id in assets.first { $0.localIdentifier == id } })
            } label: {
                Text(verbatim: "\(String(localized: .Common.commonDone)) (\(selected.count))")
            }
            .buttonStyle(WontonButtonStyle(kind: .primary))
            .padding(.horizontal, WontonTheme.spacing.pageInset)
            .padding(.vertical, WontonTheme.spacing.s8)
            .accessibilityIdentifier("attachDone")
        }
    }

    private func tab(_ icon: WontonIcon, _ title: String, isCurrent: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            VStack(spacing: WontonTheme.spacing.s4) {
                icon.view()
                Text(title).wontonText(WontonTheme.typography.caption).lineLimit(1)
            }
            .foregroundStyle(Color(isCurrent ? colors.accentInk : colors.textSecondary))
            .frame(maxWidth: .infinity, minHeight: WontonTheme.size.target)
            .contentShape(Rectangle())
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
            Circle().fill(order == nil ? Color.black.opacity(WontonTheme.ratio.scrim / 2) : Color(colors.accentInk))
            Circle().strokeBorder(.white, lineWidth: 1.5)
            if let order {
                Text(verbatim: "\(order)")
                    .wontonText(WontonTheme.typography.caption)
                    .fontWeight(.semibold)
                    .foregroundStyle(Color(colors.canvas))
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
    static func data(_ asset: PHAsset) async -> (Data, UTType?)? {
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
