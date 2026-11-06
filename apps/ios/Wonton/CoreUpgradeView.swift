import SwiftUI
import WontonClient

/// 升级家里的服务器（ADR-0025）：进展和结论都在核心，这里渲染，并把主人的操作发回去。
struct CoreUpgradeView: View {
    let model: PluginsModel
    @State private var isConfirming = false
    @State private var showsDetails = false
    @Environment(\.wontonColors) private var colors

    private var upgrade: CoreUpgradeViewModel { model.viewModel.coreUpgrade }

    var body: some View {
        content
            .wontonPushedScreen(String(localized: .coreUpgradeTitle))
            .onAppear { model.refresh(.refreshCoreUpgrade(manual: false)) }
            .onChange(of: upgrade.offer?.digest) { isConfirming = false }
    }

    @ViewBuilder private var content: some View {
        if let status = upgrade.status {
            page(status)
        } else if let problem = upgrade.problem {
            ProblemView(
                problem: Problem(title: failureSummary(problem.reason), detail: problem.message)
            ) {
                model.send(.refreshCoreUpgrade(manual: true))
            }
        } else {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func page(_ status: CoreUpgradeStatus) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
                if let problem = upgrade.problem {
                    Text(problem.message)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.danger))
                }
                DetailRow(title: String(localized: .coreUpgradeCurrentVersion), value: status.current.label)
                CoreUpgradeStageCard(model: model, stage: upgrade.stage, target: status.run?.to)
                if upgrade.stage.isSettled {
                    checkSection(status.check)
                }
                details(status)
            }
            .padding(WontonTheme.spacing.pageInset)
        }
    }

    @ViewBuilder private func checkSection(_ check: CoreUpgradeCheck) -> some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            if let offer = upgrade.offer {
                offerCard(offer)
            } else {
                CoreUpgradeCheckRow(check: check)
            }
            if check.canCheck {
                Button {
                    model.send(.refreshCoreUpgrade(manual: true))
                } label: {
                    HStack(spacing: WontonTheme.spacing.s8) {
                        if upgrade.refreshing { ProgressView() }
                        Text(
                            upgrade.refreshing ? String(localized: .coreUpgradeChecking)
                                : String(localized: .coreUpgradeCheckForUpdate)
                        )
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                .disabled(upgrade.refreshing)
            }
        }
    }

    private func offerCard(_ offer: CoreUpgradeOffer) -> some View {
        let name = offer.label
        return VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
            Text(
                isConfirming ? String(localized: .coreUpgradeConfirmVersion(name))
                    : String(localized: .coreUpgradeNewVersion(name))
            )
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            if isConfirming, !offer.migrates.isEmpty {
                Text(.coreUpgradeMigratesWarning)
                    .wontonText(WontonTheme.typography.body)
                    .foregroundStyle(Color(colors.danger))
            }
            if !isConfirming, upgrade.offerSkipped {
                Text(.coreUpgradeSkipped)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            offerButtons(offer)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private func offerButtons(_ offer: CoreUpgradeOffer) -> some View {
        if isConfirming {
            Button {
                model.send(.startCoreUpgrade(digest: offer.digest))
            } label: {
                Text(.coreUpgradeConfirmUpgrade).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle())
            Button {
                isConfirming = false
            } label: {
                Text(.coreUpgradeDeclineUpgrade).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle(kind: .secondary))
        } else {
            Button {
                isConfirming = true
            } label: {
                Text(.coreUpgradeUpgrade).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle())
            if !upgrade.offerSkipped {
                Button {
                    model.send(.skipCoreUpgrade(digest: offer.digest))
                } label: {
                    Text(.coreUpgradeSkipVersion).frame(maxWidth: .infinity)
                }
                .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
        }
    }

    @ViewBuilder private func details(_ status: CoreUpgradeStatus) -> some View {
        let rows = status.detailRows(offer: upgrade.offer, stage: upgrade.stage)
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
            Button {
                showsDetails.toggle()
            } label: {
                HStack {
                    Text(.commonDetails)
                    Spacer(minLength: 0)
                    WontonIcon.forward.view(size: WontonTheme.size.iconSmall)
                        .rotationEffect(.degrees(showsDetails ? 90 : 0))
                }
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.textSecondary))
                .frame(minHeight: WontonTheme.size.target)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if showsDetails {
                ForEach(rows, id: \.title) { row in
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                        Text(row.title)
                            .wontonText(WontonTheme.typography.caption)
                            .foregroundStyle(Color(colors.textSecondary))
                        Text(row.value)
                            .wontonText(WontonTheme.typography.supporting)
                            .foregroundStyle(Color(colors.textPrimary))
                            .textSelection(.enabled)
                    }
                }
            }
        }
    }
}

/// 这次或上次升级走到哪了。
private struct CoreUpgradeStageCard: View {
    let model: PluginsModel
    let stage: CoreUpgradeStage
    let target: CoreUpgradeTarget?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        if let line {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s12) {
                HStack(spacing: WontonTheme.spacing.s12) {
                    if let icon = line.icon {
                        icon.view().foregroundStyle(Color(line.tint(colors)))
                    } else {
                        ProgressView()
                    }
                    VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                        Text(line.title)
                            .wontonText(WontonTheme.typography.itemTitle)
                            .foregroundStyle(Color(colors.textPrimary))
                        if let detail = line.detail {
                            Text(detail)
                                .wontonText(WontonTheme.typography.supporting)
                                .foregroundStyle(Color(colors.textSecondary))
                        }
                    }
                }
                buttons
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder private var buttons: some View {
        switch stage {
        case .inProgress(_, canCancel: true):
            Button {
                model.send(.cancelCoreUpgrade)
            } label: {
                Text(.coreUpgradeCancelUpgrade).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle(kind: .danger))
        case .uncertain:
            Button {
                model.send(.refreshCoreUpgrade(manual: false))
            } label: {
                Text(.coreUpgradeCheckAgain).frame(maxWidth: .infinity)
            }
            .buttonStyle(WontonButtonStyle(kind: .secondary))
        default:
            EmptyView()
        }
    }

    private var line: StageLine? {
        let upgrading = target.map { String(localized: .coreUpgradeUpgradingTo($0.label)) }
            ?? String(localized: .coreUpgradeUpgrading)
        switch stage {
        case .idle:
            return nil
        case .requesting:
            return StageLine(title: String(localized: .coreUpgradeRequestingUpgrade))
        case let .inProgress(phase, _):
            return StageLine(title: upgrading, detail: phaseTitle(phase))
        case .reconnecting:
            return StageLine(title: upgrading, detail: String(localized: .coreUpgradeReconnecting))
        case let .uncertain(problem):
            return StageLine(
                title: String(localized: .coreUpgradeUncertain),
                detail: failureSummary(problem.reason),
                icon: .warningFill,
                tint: \.warning
            )
        case let .completed(target):
            return StageLine(
                title: String(localized: .coreUpgradeCompletedTo(target.label)),
                icon: .checkCircleFill,
                tint: \.success
            )
        case let .rolledBack(version, problem):
            return StageLine(
                title: String(localized: .coreUpgradeRolledBackTo(version.label)),
                detail: failureSummary(problem.reason),
                icon: .warningFill,
                tint: \.warning
            )
        case let .failed(problem):
            return StageLine(
                title: String(localized: .coreUpgradeFailedNoChanges),
                detail: failureSummary(problem.reason),
                icon: .errorFill,
                tint: \.danger
            )
        case .canceled:
            return StageLine(title: String(localized: .coreUpgradeCanceled), icon: .info, tint: \.textSecondary)
        }
    }
}

private struct StageLine {
    let title: String
    var detail: String?
    /// 没有图标时转圈。
    var icon: WontonIcon?
    var tint: KeyPath<ThemeColors, Rgb> = \.textSecondary

    func tint(_ colors: ThemeColors) -> Rgb { colors[keyPath: tint] }
}

/// 没有可以升级的版本时，渠道上的情况。
private struct CoreUpgradeCheckRow: View {
    let check: CoreUpgradeCheck
    @Environment(\.wontonColors) private var colors

    var body: some View {
        switch check {
        case .notInstalled:
            label(String(localized: .coreUpgradeCheckNotInstalled))
        case .pinned:
            label(String(localized: .coreUpgradeCheckPinned))
        case .notChecked, .checking:
            label(String(localized: .coreUpgradeCheckChecking))
        case .upToDate:
            label(String(localized: .coreUpgradeCheckUpToDate))
        case let .downloading(_, progress):
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                label(String(localized: .coreUpgradeCheckDownloading))
                if let progress, progress.totalBytes > 0 {
                    ProgressView(value: Double(progress.doneBytes), total: Double(progress.totalBytes))
                    Text(progress.label)
                        .wontonText(WontonTheme.typography.caption)
                        .foregroundStyle(Color(colors.textSecondary))
                }
            }
        case let .downloadFailed(_, problem), let .failed(problem):
            Text(failureSummary(problem.reason))
                .wontonText(WontonTheme.typography.body)
                .foregroundStyle(Color(colors.danger))
        case .ready:
            EmptyView()
        case .other:
            label(String(localized: .coreUpgradeCheckOther))
        }
    }

    private func label(_ text: String) -> some View {
        Text(text)
            .wontonText(WontonTheme.typography.body)
            .foregroundStyle(Color(colors.textSecondary))
    }
}

private struct DetailRow: View {
    let title: String
    let value: String
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack {
            Text(title).foregroundStyle(Color(colors.textSecondary))
            Spacer(minLength: WontonTheme.spacing.s12)
            Text(value).foregroundStyle(Color(colors.textPrimary))
        }
        .wontonText(WontonTheme.typography.body)
    }
}

private struct CoreUpgradeDetail {
    let title: String
    let value: String
}

extension CoreUpgradeStage {
    /// 没有升级在进行，可以看渠道上的版本。
    fileprivate var isSettled: Bool {
        switch self {
        case .requesting, .inProgress, .reconnecting: return false
        case .idle, .uncertain, .completed, .rolledBack, .failed, .canceled: return true
        }
    }

    fileprivate var problem: CoreUpgradeProblem? {
        switch self {
        case let .uncertain(problem), let .rolledBack(_, problem), let .failed(problem): return problem
        case .idle, .requesting, .inProgress, .reconnecting, .completed, .canceled: return nil
        }
    }
}

extension CoreUpgradeCheck {
    fileprivate var canCheck: Bool {
        switch self {
        case .notInstalled, .pinned, .other: return false
        case .notChecked, .checking, .upToDate, .downloading, .ready, .downloadFailed, .failed: return true
        }
    }

    fileprivate var problem: CoreUpgradeProblem? {
        switch self {
        case let .downloadFailed(_, problem), let .failed(problem): return problem
        default: return nil
        }
    }
}

extension CoreUpgradeStatus {
    fileprivate func detailRows(offer: CoreUpgradeOffer?, stage: CoreUpgradeStage) -> [CoreUpgradeDetail] {
        let values: [(String, String?)] = [
            (String(localized: .coreUpgradeDetailReason), stage.problem?.message ?? check.problem?.message),
            (String(localized: .coreUpgradeDetailChannel), channel),
            (String(localized: .coreUpgradeCurrentVersion), current.digest),
            (String(localized: .coreUpgradeDetailNewVersion), offer?.digest ?? run?.to.digest),
            (
                String(localized: .coreUpgradeDetailMigrates),
                offer.flatMap { $0.migrates.isEmpty ? nil : $0.migrates.formatted(.list(type: .and)) }
            )
        ]
        return values.compactMap { title, value in value.map { CoreUpgradeDetail(title: title, value: $0) } }
    }
}

extension CoreVersion {
    fileprivate var label: String { version ?? digest.map(shortDigest) ?? String(localized: .coreUpgradeDevVersion) }
}

extension CoreUpgradeTarget {
    fileprivate var label: String { version ?? shortDigest(digest) }
}

extension CoreUpgradeOffer {
    fileprivate var label: String { version ?? shortDigest(digest) }
}

extension CoreUpgradeProgress {
    fileprivate var label: String {
        "\(doneBytes.formatted(.byteCount(style: .file))) / \(totalBytes.formatted(.byteCount(style: .file)))"
    }
}

/// 失败原因的一句话，显示在升级页上。核心写的详细原因放详情。
private func failureSummary(_ reason: CoreUpgradeFailure) -> String {
    switch reason {
    case .transportUnavailable: String(localized: .problemTransportUnavailableTitle)
    case .unauthorized: String(localized: .problemUnauthorizedTitle)
    case .changed: String(localized: .coreUpgradeFailureChanged)
    case .notReady: String(localized: .coreUpgradeFailureNotReady)
    case .busy: String(localized: .coreUpgradeFailureBusy)
    case .tooLate: String(localized: .coreUpgradeFailureTooLate)
    case .registryUnavailable: String(localized: .coreUpgradeFailureRegistryUnavailable)
    case .noSpace: String(localized: .coreUpgradeFailureNoSpace)
    case .initFailed: String(localized: .coreUpgradeFailureInitFailed)
    case .unhealthy: String(localized: .coreUpgradeFailureUnhealthy)
    case .crashed: String(localized: .coreUpgradeFailureCrashed)
    case .timedOut: String(localized: .coreUpgradeFailureTimedOut)
    case .updaterFailed: String(localized: .coreUpgradeFailureUpdaterFailed)
    case .upstream: String(localized: .coreUpgradeFailureUpstream)
    case .protocol, .other: String(localized: .coreUpgradeFailureProtocol)
    }
}

/// 升级进行到的阶段，显示在正在升级的那一行下面。
private func phaseTitle(_ phase: CoreUpgradePhase) -> String {
    switch phase {
    case .pulling: String(localized: .coreUpgradePhasePulling)
    case .backingUp: String(localized: .coreUpgradePhaseBackingUp)
    case .switching: String(localized: .coreUpgradePhaseSwitching)
    case .starting: String(localized: .coreUpgradePhaseStarting)
    case .observing: String(localized: .coreUpgradePhaseObserving)
    case .rollingBack: String(localized: .coreUpgradePhaseRollingBack)
    case .other: String(localized: .coreUpgradePhaseOther)
    }
}

private func shortDigest(_ digest: String) -> String {
    String(digest.replacingOccurrences(of: "sha256:", with: "").prefix(12))
}
