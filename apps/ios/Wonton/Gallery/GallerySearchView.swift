import SwiftUI
import WontonClient

/// 时间筛选的几档；“自定”时用 `from`、`until`。
enum GalleryTimeFilter: Hashable {
    case any
    case month
    case thisYear
    case lastYear
    case custom

    var title: String {
        switch self {
        case .any: GalleryText.timeAny
        case .month: GalleryText.timeMonth
        case .thisYear: GalleryText.timeThisYear
        case .lastYear: GalleryText.timeLastYear
        case .custom: GalleryText.timeCustom
        }
    }
}

/// 搜索页填的条件。条件都空就清掉搜索，核心的规则是“都空不能搜”。
struct GallerySearchDraft: Equatable {
    var text = ""
    var place = ""
    var people: [String] = []
    var time = GalleryTimeFilter.any
    var from = Calendar.current.date(byAdding: .month, value: -1, to: .now) ?? .now
    var until = Date.now
    var kind: MediaKind?
    var favorite = false

    private func milliseconds(_ date: Date) -> UInt64 { UInt64(max(0, date.timeIntervalSince1970) * 1000) }

    private var range: (from: Date, to: Date)? {
        let calendar = Calendar.current
        let now = Date.now
        switch time {
        case .any:
            return nil
        case .month:
            return (calendar.date(byAdding: .day, value: -30, to: now) ?? now, now)
        case .thisYear:
            return (calendar.dateInterval(of: .year, for: now)?.start ?? now, now)
        case .lastYear:
            guard let year = calendar.date(byAdding: .year, value: -1, to: now),
                  let interval = calendar.dateInterval(of: .year, for: year) else { return nil }
            return (interval.start, interval.end.addingTimeInterval(-1))
        case .custom:
            let start = calendar.startOfDay(for: min(from, until))
            let end = calendar.startOfDay(for: max(from, until)).addingTimeInterval(24 * 60 * 60 - 1)
            return (start, end)
        }
    }

    var query: PhotoSearch {
        let trimmed = { (value: String) -> String? in
            let text = value.trimmingCharacters(in: .whitespacesAndNewlines)
            return text.isEmpty ? nil : text
        }
        let span = range
        return PhotoSearch(
            text: trimmed(text), people: people, place: trimmed(place),
            from: span.map { milliseconds($0.from) }, to: span.map { milliseconds($0.to) },
            kind: kind, favorite: favorite ? true : nil
        )
    }

    var isEmpty: Bool {
        let filled = query
        return filled.text == nil && filled.place == nil && people.isEmpty && time == .any && kind == nil && !favorite
    }
}

/// 搜索：描述画面、按人、按地点、按时间筛，结果是一张网格。
struct GallerySearchView: View {
    let model: GalleryModel
    let selection: GallerySelection
    let columns: Int
    @Binding var draft: GallerySearchDraft
    let open: (PhotoTile) -> Void
    @State private var showsDates = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
                    WontonSearchField(text: $draft.text, prompt: GalleryText.searchPrompt)
                        .onSubmit(search)
                        .accessibilityIdentifier("gallery.search.text")
                    people
                    filters
                }
                .padding(.horizontal, WontonTheme.spacing.pageInset)
                .padding(.vertical, WontonTheme.spacing.s12)
                if let result = model.view.search {
                    GalleryPageGrid(
                        page: result.page, origin: .search, columns: columns, model: model, selection: selection,
                        open: open, retry: search,
                        empty: { GalleryEmptyView(icon: .search, title: GalleryText.emptySearch) { EmptyView() } }
                    )
                }
            }
            .galleryDragSelect(selection)
        }
        .scrollDisabled(selection.dragging)
        .scrollDismissesKeyboard(.immediately)
        .contentMargins(.bottom, GalleryChrome.bottomClearance, for: .scrollContent)
        .onChange(of: draft.people) { _, _ in search() }
        .onChange(of: draft.time) { _, _ in search() }
        .onChange(of: draft.kind) { _, _ in search() }
        .onChange(of: draft.favorite) { _, _ in search() }
        .onChange(of: draft.text) { _, text in if text.isEmpty { search() } }
        .sheet(isPresented: $showsDates, onDismiss: search) { dates }
    }

    private func search() {
        if draft.isEmpty {
            if model.view.search != nil { model.send(.clearSearch) }
        } else {
            model.send(.search(query: draft.query))
        }
    }

    @ViewBuilder private var people: some View {
        let list = model.view.people
        if let failure = list.problem, list.people.isEmpty {
            ProblemRow(problem: failure.galleryProblem, retry: model.retry)
        } else if !list.people.isEmpty {
            ScrollView(.horizontal) {
                HStack(alignment: .top, spacing: WontonTheme.spacing.s12) {
                    ForEach(list.people, id: \.id) { person in
                        GalleryPersonChip(
                            person: person, model: model, selected: draft.people.contains(person.id)
                        ) {
                            if let index = draft.people.firstIndex(of: person.id) {
                                draft.people.remove(at: index)
                            } else {
                                draft.people.append(person.id)
                            }
                        }
                    }
                }
            }
            .scrollIndicators(.hidden)
            .accessibilityLabel(GalleryText.infoPeople)
        }
    }

    private var filters: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            HStack(spacing: WontonTheme.spacing.s8) {
                WontonIcon.globe.view(size: WontonTheme.size.iconSmall)
                    .foregroundStyle(Color(colors.textSecondary))
                TextField(GalleryText.place, text: $draft.place)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.textPrimary))
                    .submitLabel(.search)
                    .onSubmit(search)
            }
            .padding(.horizontal, WontonTheme.spacing.s12)
            .frame(minHeight: WontonTheme.size.target)
            .background(
                RoundedRectangle(cornerRadius: WontonTheme.radii.control).fill(Color(colors.surfaceSubtle))
            )
            ScrollView(.horizontal) {
                HStack(spacing: WontonTheme.spacing.s8) {
                    Menu {
                        ForEach([GalleryTimeFilter.any, .month, .thisYear, .lastYear], id: \.self) { option in
                            Button(option.title) { draft.time = option }
                        }
                        Button(GalleryTimeFilter.custom.title) {
                            draft.time = .custom
                            showsDates = true
                        }
                    } label: {
                        GalleryFilterLabel(icon: .clock, title: timeTitle, selected: draft.time != .any)
                    }
                    .buttonStyle(.plain)
                    WontonChip(title: GalleryText.kindPhoto, isSelected: draft.kind == .photo) {
                        draft.kind = draft.kind == .photo ? nil : .photo
                    }
                    WontonChip(title: GalleryText.kindVideo, isSelected: draft.kind == .video) {
                        draft.kind = draft.kind == .video ? nil : .video
                    }
                    WontonChip(title: GalleryText.favorites, isSelected: draft.favorite) {
                        draft.favorite.toggle()
                    }
                }
            }
            .scrollIndicators(.hidden)
        }
    }

    private var timeTitle: String {
        guard draft.time == .custom else { return draft.time.title }
        let style = Date.FormatStyle.dateTime.year().month().day()
        return "\(min(draft.from, draft.until).formatted(style)) – \(max(draft.from, draft.until).formatted(style))"
    }

    private var dates: some View {
        NavigationStack {
            Form {
                DatePicker(GalleryText.timeFrom, selection: $draft.from, displayedComponents: .date)
                DatePicker(GalleryText.timeTo, selection: $draft.until, displayedComponents: .date)
            }
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(String(localized: .Common.commonDone)) { showsDates = false }
                }
            }
        }
        .presentationDetents([.height(240)])
    }
}

/// 时间筛选的胶囊：样子同 `WontonChip`，前面多一个图标。
private struct GalleryFilterLabel: View {
    let icon: WontonIcon
    let title: String
    let selected: Bool
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(spacing: WontonTheme.spacing.s4) {
            icon.view(size: WontonTheme.size.iconTiny)
            Text(title).wontonText(WontonTheme.typography.supporting)
        }
        .foregroundStyle(Color(selected ? colors.onPrimary : colors.textPrimary))
        .padding(.horizontal, WontonTheme.spacing.s12)
        .frame(minHeight: WontonTheme.size.chip)
        .background {
            if selected {
                Color.clear.emphasisFill(Capsule())
            } else {
                Capsule().fill(Color(colors.surfaceSubtle))
            }
        }
        .frame(minHeight: WontonTheme.size.target)
        .contentShape(Rectangle())
    }
}

/// 人物条里的一个头像：点一下按这个人筛，再点取消。
private struct GalleryPersonChip: View {
    let person: PhotoPerson
    let model: GalleryModel
    let selected: Bool
    let action: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Button(action: action) {
            VStack(spacing: WontonTheme.spacing.s4) {
                GalleryImage(
                    request: GalleryImageRequest(
                        scope: model.cacheScope, subject: .person(id: person.id), maxPixel: 192
                    ),
                    thumbhash: nil, core: model.core
                )
                .frame(width: 60, height: 60)
                .clipShape(Circle())
                .padding(3)
                .overlay {
                    Circle().strokeBorder(selected ? Color(colors.primary) : Color.clear, lineWidth: 2)
                }
                .overlay(alignment: .bottomTrailing) {
                    if selected {
                        WontonIcon.checkCircleFill.view(size: WontonTheme.size.iconSmall)
                            .foregroundStyle(Color(colors.primary))
                            .background(Circle().fill(Color(colors.canvas)))
                    }
                }
                Text(person.name)
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.textPrimary))
                    .lineLimit(1)
                    .frame(maxWidth: 66)
            }
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}
