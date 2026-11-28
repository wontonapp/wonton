import SwiftUI
import WontonClient

extension WontonClient.MediaFailure {
    /// 类别由核心给，文字按类别写在这里；原样的原因和类别名收进“详情”。
    var galleryProblem: Problem {
        let detail = "\(String(describing: code)): \(message)"
        switch code {
        case .transportUnavailable:
            return Problem(
                title: String(localized: .Problem2.problemTransportUnavailableTitle),
                nextStep: String(localized: .Problem3.problemTransportUnavailableNextStep),
                detail: detail
            )
        case .pluginUnavailable:
            return Problem(
                title: GalleryText.problemPluginTitle, nextStep: GalleryText.problemPluginNext, detail: detail
            )
        case .permissionDenied:
            return Problem(
                title: GalleryText.problemDeniedTitle, nextStep: GalleryText.problemDeniedNext, detail: detail
            )
        case .authenticationRequired:
            return Problem(
                title: GalleryText.problemAuthTitle, nextStep: GalleryText.problemRetryNext, detail: detail
            )
        case .rateLimited:
            return Problem(
                title: GalleryText.problemBusyTitle, nextStep: GalleryText.problemRetryNext, detail: detail
            )
        case .mediaGone:
            return Problem(title: GalleryText.problemGoneTitle, detail: detail)
        case .upstreamError:
            return Problem(
                title: GalleryText.problemUpstreamTitle, nextStep: GalleryText.problemRetryNext, detail: detail
            )
        case .sourceRequired:
            return Problem(title: GalleryText.sourceTitle, nextStep: GalleryText.sourceNext, detail: detail)
        default:
            return Problem(
                title: GalleryText.problemGenericTitle, nextStep: GalleryText.problemRetryNext, detail: detail
            )
        }
    }

    /// 相册应用没装或没在跑：给一个去商店的出口。
    var suggestsStore: Bool { code == .pluginUnavailable }
}

/// 整块取不到：一句话说发生了什么、能做什么，配重试；类别和原因在收起的“详情”里。
struct GalleryProblemView: View {
    let failure: WontonClient.MediaFailure
    let retry: () -> Void
    var openStore: (() -> Void)?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        let problem = failure.galleryProblem
        VStack(spacing: WontonTheme.spacing.s8) {
            WontonIcon.warning.view().foregroundStyle(Color(colors.textSecondary))
            Text(problem.title)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            if let nextStep = problem.nextStep {
                Text(nextStep)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textSecondary))
            }
            HStack(spacing: WontonTheme.spacing.s8) {
                Button(String(localized: .Common.commonRetry), action: retry)
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
                if failure.suggestsStore, let openStore {
                    Button(GalleryText.openStore, action: openStore)
                        .buttonStyle(WontonButtonStyle(kind: .primary))
                }
            }
            .padding(.vertical, WontonTheme.spacing.s8)
            if let detail = problem.detail { ProblemDetail(text: detail) }
        }
        .multilineTextAlignment(.center)
        .padding(WontonTheme.spacing.s32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// 空的一块：一句话，能做的事放在下面。
struct GalleryEmptyView<Actions: View>: View {
    let icon: WontonIcon
    let title: String
    @ViewBuilder let actions: Actions
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            icon.view(size: WontonTheme.size.taskIcon).foregroundStyle(Color(colors.textSecondary))
            Text(title)
                .wontonText(WontonTheme.typography.itemTitle)
                .foregroundStyle(Color(colors.textPrimary))
            actions
        }
        .multilineTextAlignment(.center)
        .padding(WontonTheme.spacing.s32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// 网格的骨架：占住格子的位置，取到了原地换成照片。
struct GalleryGridSkeleton: View {
    let columns: Int
    var rows = 6
    @State private var isDim = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.wontonColors) private var colors

    var body: some View {
        LazyVGrid(columns: GalleryGrid.columns(columns), spacing: GalleryGrid.gap) {
            ForEach(0 ..< columns * rows, id: \.self) { _ in
                Color(colors.surfaceSubtle).aspectRatio(1, contentMode: .fit)
            }
        }
        .opacity(isDim ? WontonTheme.ratio.secondaryContent : 1)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(.Problem1.problemLoading))
        .onAppear {
            guard !reduceMotion else { return }
            withAnimation(.easeInOut(duration: 0.9).repeatForever()) { isDim = true }
        }
    }
}
