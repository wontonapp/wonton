import SwiftUI
import WontonClient

/// 时间线：按月分桶、按日分组的照片网格。月滚到了才向核心要条目；捏合换列数。
struct GalleryTimelineView: View {
    let model: GalleryModel
    let selection: GallerySelection
    let columns: Int
    let open: (PhotoTile) -> Void
    let openStore: (() -> Void)?
    @Environment(\.wontonColors) private var colors

    private var timeline: PhotoTimeline { model.view.timeline }

    var body: some View {
        if timeline.months.isEmpty {
            if let failure = timeline.problem {
                GalleryProblemView(failure: failure, retry: model.retry, openStore: openStore)
            } else if timeline.loading || !model.opened {
                ScrollView { GalleryGridSkeleton(columns: columns) }.scrollDisabled(true)
            } else {
                GalleryEmptyView(icon: .image, title: GalleryText.emptyTimeline) {
                    Button(GalleryText.refresh, action: model.retry)
                        .buttonStyle(WontonButtonStyle(kind: .secondary))
                }
            }
        } else {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    if let failure = timeline.problem {
                        ProblemRow(problem: failure.galleryProblem, retry: model.retry)
                            .padding(WontonTheme.spacing.pageInset)
                    }
                    ForEach(timeline.months, id: \.month) { month in
                        monthBlock(month)
                    }
                }
                .galleryDragSelect(selection)
            }
            .scrollDisabled(selection.dragging)
            .contentMargins(.bottom, GalleryChrome.bottomClearance, for: .scrollContent)
            .refreshable { model.retry() }
        }
    }

    @ViewBuilder private func monthBlock(_ month: PhotoMonth) -> some View {
        if month.days.isEmpty {
            header(GalleryFormat.month(month.month))
            if let failure = month.problem {
                ProblemRow(problem: failure.galleryProblem, retry: model.retry)
                    .padding(WontonTheme.spacing.pageInset)
            } else {
                // 还没取条目的月：按张数占住大致的高度，滚到了再取，取到后原地换成照片。
                GalleryGridSkeleton(columns: columns, rows: placeholderRows(month))
                    .onAppear { model.send(.openMonth(month: month.month)) }
            }
        } else {
            ForEach(month.days, id: \.date) { day in
                header(GalleryFormat.day(day.date))
                GalleryTiles(tiles: day.tiles, columns: columns, model: model, selection: selection, open: open)
            }
            if let failure = month.problem {
                ProblemRow(problem: failure.galleryProblem) {
                    model.send(.loadMoreMonth(month: month.month))
                }
                .padding(WontonTheme.spacing.pageInset)
            } else if month.hasMore {
                ProgressView()
                    .frame(maxWidth: .infinity)
                    .padding(WontonTheme.spacing.s16)
                    .onAppear { model.send(.loadMoreMonth(month: month.month)) }
            }
        }
    }

    private func placeholderRows(_ month: PhotoMonth) -> Int {
        let rows = (Int(month.count) + columns - 1) / max(columns, 1)
        return min(max(rows, 1), 12)
    }

    private func header(_ title: String) -> some View {
        Text(title)
            .wontonText(WontonTheme.typography.itemTitle)
            .foregroundStyle(Color(colors.textPrimary))
            .padding(.horizontal, WontonTheme.spacing.pageInset)
            .padding(.top, WontonTheme.spacing.s24)
            .padding(.bottom, WontonTheme.spacing.s8)
            .accessibilityAddTraits(.isHeader)
    }
}
