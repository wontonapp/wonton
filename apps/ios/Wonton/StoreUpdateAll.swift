import SwiftUI
import WontonClient

/// Kept outside the catalog scroll so a batch started far down the store remains visible.
struct StoreUpdateAllResults: View {
    let model: PluginsModel
    let batch: StoreUpdateBatch
    let maximumHeight: CGFloat
    let review: (PluginTask) -> Void
    let checkInstalled: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            header
            ViewThatFits(in: .vertical) {
                rows.fixedSize(horizontal: false, vertical: true)
                ScrollView { rows }
                    .scrollBounceBehavior(.basedOnSize)
            }
        }
        .padding(.horizontal, WontonTheme.spacing.pageInset)
        .padding(.vertical, WontonTheme.spacing.s8)
        .frame(maxHeight: maximumHeight, alignment: .top)
        .overlay(alignment: .bottom) { Divider() }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("store-update-batch")
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline, spacing: WontonTheme.spacing.s8) {
            Group {
                if batch.finished {
                    Text(.StoreBatch.storeBatchSummary(
                        Int(batch.succeeded), Int(batch.failed), Int(batch.needsAttention)
                    ))
                } else {
                    let completed = Int(batch.succeeded) + Int(batch.failed)
                    Text(.StoreBatch.storeBatchProgress(completed, batch.items.count))
                }
            }
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textPrimary))
            .accessibilityAddTraits(.isHeader)
            Spacer(minLength: 0)
            if batch.finished {
                Button(String(localized: .StoreBatch.storeBatchClear)) {
                    model.send(.plugins(action: .dismissStoreUpdates))
                }
                .buttonStyle(WontonSmallButtonStyle())
                .accessibilityIdentifier("store-update-clear")
            }
        }
    }

    private var rows: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            ForEach(batch.items, id: \.app.pluginId) { item in
                row(item)
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier("store-update-\(item.app.pluginId)")
            }
        }
        .padding(.vertical, WontonTheme.spacing.s4)
    }

    @ViewBuilder private func row(_ item: StoreUpdateItem) -> some View {
        switch item.state {
        case let .task(task):
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                TaskRow(
                    model: model, task: task, review: { review(task) },
                    checkInstalled: checkInstalled, showsDismiss: false
                )
                Text(.StoreBatch.storeBatchTargetVersion(item.app.version))
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.textSecondary))
                    .padding(.leading, WontonTheme.size.taskIcon + WontonTheme.spacing.s12)
            }
        case .queued:
            waitingRow(item.app, status: String(localized: .StoreBatch.storeBatchQueued))
        case .checking:
            waitingRow(
                item.app, status: String(localized: .StoreBatch.storeBatchChecking), checking: true
            )
        case .needsDecision:
            waitingRow(
                item.app, status: String(localized: .StoreBatch.storeBatchNeedsDecision), needsReview: true
            )
        }
    }

    private func waitingRow(
        _ app: UpdatablePlugin, status: String, checking: Bool = false, needsReview: Bool = false
    ) -> some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            PluginIconView(icon: app.icon, size: WontonTheme.size.taskIcon, model: model)
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(app.title)
                    .wontonText(WontonTheme.typography.itemTitle)
                    .foregroundStyle(Color(colors.textPrimary))
                Text(.StoreBatch.storeBatchTargetVersion(app.version))
                    .wontonText(WontonTheme.typography.caption)
                    .foregroundStyle(Color(colors.textSecondary))
                Text(status)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(needsReview ? colors.warning : colors.textSecondary))
            }
            Spacer(minLength: 0)
            if checking { ProgressView().accessibilityLabel(status) }
            if needsReview {
                Button(String(localized: .StoreBatch.storeBatchReview)) {
                    model.send(.plugins(action: .reviewStoreUpdate(pluginId: app.pluginId)))
                }
                .buttonStyle(WontonSmallButtonStyle())
                .accessibilityLabel(String(localized: .StoreBatch.storeBatchReviewApp(app.title)))
            }
        }
    }
}

/// One initial confirmation lists every app and target version; plain updates need no further prompt.
struct StoreUpdateAllConfirmation: View {
    let model: PluginsModel
    let apps: [UpdatablePlugin]
    let close: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
                    ForEach(apps, id: \.pluginId) { app in
                        HStack(spacing: WontonTheme.spacing.s12) {
                            PluginIconView(icon: app.icon, size: WontonTheme.size.taskIcon, model: model)
                            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                                Text(app.title)
                                    .wontonText(WontonTheme.typography.itemTitle)
                                    .foregroundStyle(Color(colors.textPrimary))
                                Text(.StoreBatch.storeBatchTargetVersion(app.version))
                                    .wontonText(WontonTheme.typography.supporting)
                                    .foregroundStyle(Color(colors.textSecondary))
                            }
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
                .padding(WontonTheme.spacing.pageInset)
            }
            Button(String(localized: .Store.storeUpdateAll)) {
                close()
                model.send(.plugins(action: .upgradeAll))
            }
            .buttonStyle(WontonButtonStyle())
            .disabled(apps.isEmpty || model.viewModel.updateBatch?.finished == false)
            .padding(WontonTheme.spacing.pageInset)
            .accessibilityIdentifier("store-update-confirm")
        }
        .wontonModalScreen(String(localized: .Store.storeUpdateAllTitle(count: apps.count)))
    }
}
