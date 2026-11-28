import SwiftUI
import WontonClient

enum GalleryChrome {
    /// 网格底下给悬浮的分区条、操作条让出的高度。
    static let bottomClearance: CGFloat = 96
}

/// 相册的三个分区。
enum GallerySection: Hashable, CaseIterable {
    case timeline
    case albums
    case search

    var title: String {
        switch self {
        case .timeline: GalleryText.sectionTimeline
        case .albums: GalleryText.sectionAlbums
        case .search: GalleryText.sectionSearch
        }
    }

    func icon(selected: Bool) -> WontonIcon {
        switch self {
        case .timeline: selected ? .imageFill : .image
        case .albums: selected ? .folderFill : .folder
        case .search: .search
        }
    }
}

/// 底下悬浮的分区条：一层玻璃的胶囊，里面三个分区。
struct GallerySectionBar: View {
    @Binding var section: GallerySection
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s4) {
            ForEach(GallerySection.allCases, id: \.self) { item in
                let selected = item == section
                Button { section = item } label: {
                    VStack(spacing: 2) {
                        item.icon(selected: selected).view()
                        Text(item.title)
                            .wontonText(WontonTheme.typography.caption)
                            .fontWeight(selected ? .semibold : .regular)
                    }
                    .foregroundStyle(Color(selected ? colors.textPrimary : colors.textSecondary))
                    .frame(width: 84, height: 52)
                    .background {
                        if selected { Capsule().fill(Color(colors.textPrimary).opacity(0.08)) }
                    }
                    .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(selected ? .isSelected : [])
                .accessibilityIdentifier("gallery.section.\(item)")
            }
        }
        .padding(WontonTheme.spacing.s4)
        .wontonGlassSurface(in: Capsule())
    }
}

/// 操作条里的一个操作。
struct GalleryAction: Identifiable {
    let id: String
    let icon: WontonIcon
    let title: String
    var isDanger = false
    let perform: () -> Void
}

/// 悬浮的操作条（多选时、大图页底下）：只列这个源接的操作，图标下面写名字。
struct GalleryActionBar: View {
    let actions: [GalleryAction]
    /// 有写操作发出去还没回。
    var busy = false
    /// 盖在照片上（大图页）：字用白色。
    var onPhoto = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: 0) {
            ForEach(actions) { action in
                Button(action: action.perform) {
                    VStack(spacing: 2) {
                        action.icon.view()
                        Text(action.title).wontonText(WontonTheme.typography.caption).lineLimit(1)
                    }
                    .foregroundStyle(tint(action))
                    .frame(minWidth: 76, minHeight: 52)
                    .padding(.horizontal, WontonTheme.spacing.s4)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("gallery.action.\(action.id)")
            }
            .environment(\.wontonIconMono, onPhoto)
            if busy { ProgressView().padding(.horizontal, WontonTheme.spacing.s12) }
        }
        .padding(.horizontal, WontonTheme.spacing.s8)
        .padding(.vertical, WontonTheme.spacing.s4)
        .wontonGlassSurface(in: Capsule())
    }

    private func tint(_ action: GalleryAction) -> Color {
        if action.isDanger { return Color(colors.danger) }
        return onPhoto ? .white : Color(colors.textPrimary)
    }
}

/// 写操作的结果：一条轻提示。部分没成时说清几条没成，原因收在“详情”里；能撤的带撤销。
struct GalleryEditNotice: View {
    let outcome: PhotoEditOutcome
    let canUndo: Bool
    let undo: () -> Void
    let dismiss: () -> Void
    @Environment(\.wontonColors) private var colors

    private var failedWhole: Bool { outcome.problem != nil }
    private var isClean: Bool { outcome.problem == nil && outcome.failed.isEmpty }

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
                (isClean ? WontonIcon.checkCircle : WontonIcon.warning).view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(isClean ? colors.textSecondary : colors.danger))
                VStack(alignment: .leading, spacing: 2) {
                    Text(title)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textPrimary))
                    if let nextStep = outcome.problem?.galleryProblem.title {
                        Text(nextStep)
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.textSecondary))
                    }
                }
                Spacer(minLength: WontonTheme.spacing.s8)
                if canUndo, outcome.kind == .trash, !outcome.done.isEmpty {
                    Button(GalleryText.undo, action: undo).buttonStyle(WontonSmallButtonStyle())
                }
                Button(action: dismiss) {
                    WontonIcon.close.view(size: WontonTheme.size.iconSmall)
                        .foregroundStyle(Color(colors.textSecondary))
                        .frame(width: WontonTheme.size.smallButton, height: WontonTheme.size.smallButton)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(String(localized: .Common.commonClose))
            }
            if let detail { ProblemDetail(text: detail).padding(.leading, WontonTheme.spacing.s24 + 4) }
        }
        .padding(WontonTheme.spacing.s12)
        // 提示是内容，不铺玻璃（ADR-0107）：实底加一圈细线，盖在照片上也读得清。
        .background {
            RoundedRectangle(cornerRadius: WontonTheme.radii.card, style: .continuous)
                .fill(Color(colors.surface))
                .shadow(color: .black.opacity(0.16), radius: 12, y: 4)
        }
        .overlay {
            RoundedRectangle(cornerRadius: WontonTheme.radii.card, style: .continuous)
                .strokeBorder(Color(colors.separator), lineWidth: WontonTheme.size.hairline)
        }
        .accessibilityIdentifier("gallery.editNotice")
        .task(id: outcome.done.count + outcome.failed.count) {
            // 全成了的提示自己退；没成的留着等人看。
            guard isClean else { return }
            try? await Task.sleep(for: .seconds(5))
            if !Task.isCancelled { dismiss() }
        }
    }

    private var title: String {
        if failedWhole { return GalleryText.failedEdit }
        let done = doneText(outcome.done.count)
        guard !outcome.failed.isEmpty else { return done }
        let failed = GalleryText.partFailed(outcome.failed.count)
        if outcome.done.isEmpty { return failed }
        return done + String(localized: .Common.commonSentenceSeparator) + failed
    }

    private func doneText(_ count: Int) -> String {
        switch outcome.kind {
        case .favorite: GalleryText.doneFavorite(count)
        case .unfavorite: GalleryText.doneUnfavorite(count)
        case .trash: GalleryText.doneTrash(count)
        case .restore: GalleryText.doneRestore(count)
        case .addToAlbum: GalleryText.doneAdd(count)
        case .removeFromAlbum: GalleryText.doneRemove(count)
        case .createAlbum: GalleryText.doneCreate
        case .shareAlbum: GalleryText.doneShare
        }
    }

    private var detail: String? {
        if let failure = outcome.problem { return failure.galleryProblem.detail }
        guard !outcome.failed.isEmpty else { return nil }
        return outcome.failed.map { "\($0.id) · \(String(describing: $0.failure.code)): \($0.failure.message)" }
            .joined(separator: "\n")
    }
}

/// 盖在照片上的圆钮：iOS 26 是系统的玻璃钮，更早的系统垫一层材质。
struct GalleryPhotoButton: View {
    let icon: WontonIcon
    let label: String
    var tint = Color.white
    let action: () -> Void

    var body: some View {
        control.environment(\.wontonIconMono, true)
    }

    @ViewBuilder private var control: some View {
        if #available(iOS 26, *) {
            Button(action: action) { icon.view().foregroundStyle(tint) }
                .buttonStyle(.glass)
                .buttonBorderShape(.circle)
                .controlSize(.large)
                .accessibilityLabel(label)
        } else {
            Button(action: action) {
                icon.view().foregroundStyle(tint)
                    .frame(width: 44, height: 44)
                    .background(.ultraThinMaterial, in: Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(label)
        }
    }
}

extension View {
    /// 捏合换列数：捏拢多排几列，张开少排几列；松手时定档。选着的时候不换，格子挪了位置连选会乱。
    func galleryPinch(level: Binding<Int>, selection: GallerySelection) -> some View {
        simultaneousGesture(
            MagnifyGesture().onEnded { value in
                guard !selection.dragging else { return }
                let step = value.magnification > 1.2 ? -1 : (value.magnification < 0.83 ? 1 : 0)
                let next = min(max(level.wrappedValue + step, 0), GalleryGrid.levels.count - 1)
                guard next != level.wrappedValue else { return }
                selection.forgetFrames()
                withAnimation(.snappy) { level.wrappedValue = next }
            }
        )
    }
}
