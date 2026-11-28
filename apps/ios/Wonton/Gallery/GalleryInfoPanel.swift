import SwiftUI
import WontonClient

/// 信息面板：拍摄时间、相机、地点、人物和文件。每行前面的图标说明是什么，不再写一遍标签。
struct GalleryInfoPanel: View {
    let detail: PhotoDetailView?
    let tile: PhotoTile
    let retry: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
                row(.clock, GalleryText.infoTime, [tile.takenDate.formatted(date: .complete, time: .shortened)])
                if let owner = tile.ownerLine {
                    row(.person, owner, [owner])
                }
                if let info = detail?.info {
                    fields(info)
                } else if let failure = detail?.problem {
                    ProblemRow(problem: failure.galleryProblem, retry: retry)
                } else {
                    ProgressView().frame(maxWidth: .infinity)
                }
            }
            .padding(WontonTheme.spacing.s24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Color(colors.canvas))
    }

    @ViewBuilder private func fields(_ info: PhotoInfo) -> some View {
        if let description = info.description, !description.isEmpty {
            Text(description)
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textPrimary))
        }
        let camera = [info.camera, info.lens].compactMap { $0 }
        if !camera.isEmpty { row(.device, GalleryText.infoCamera, camera) }
        if !info.place.isEmpty {
            row(.globe, GalleryText.place, [info.place.joined(separator: GalleryText.listSeparator)])
        }
        if !info.people.isEmpty {
            let names = info.people.map(\.name).joined(separator: GalleryText.listSeparator)
            row(.person, GalleryText.infoPeople, [names])
        }
        let file = [info.fileName, measure(info)].compactMap { $0 }
        if !file.isEmpty { row(.file, GalleryText.infoFile, file) }
    }

    private func measure(_ info: PhotoInfo) -> String? {
        var parts: [String] = []
        if let width = info.width, let height = info.height { parts.append("\(width) × \(height)") }
        if let size = info.size {
            parts.append(ByteCountFormatter.string(fromByteCount: Int64(clamping: size), countStyle: .file))
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    private func row(_ icon: WontonIcon, _ label: String, _ lines: [String]) -> some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s16) {
            icon.view().foregroundStyle(Color(colors.textSecondary))
            VStack(alignment: .leading, spacing: 2) {
                ForEach(Array(lines.enumerated()), id: \.offset) { index, line in
                    Text(line)
                        .wontonText(index == 0 ? WontonTheme.typography.body : WontonTheme.typography.supporting)
                        .foregroundStyle(Color(index == 0 ? colors.textPrimary : colors.textSecondary))
                        .textSelection(.enabled)
                }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(label): \(lines.joined(separator: ", "))")
    }
}
