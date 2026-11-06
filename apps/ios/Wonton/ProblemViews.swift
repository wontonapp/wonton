import SwiftUI
import WontonClient

/// 给人看的问题：`title` 说发生了什么，`nextStep` 说能做什么，`detail` 是核心写的技术细节，默认收起。类别由核心给，
/// 文字按类别写在这里。
struct Problem {
    let title: String
    var nextStep: String?
    var detail: String?
}

extension PluginProblem {
    var problem: Problem { reason.problem(detail: message) }
}

extension PluginProblem {
    /// 保存出网代理没成功：连不上时不知道到底改了没有，填错了要照着核心说的改。
    var saveProblem: Problem {
        switch reason {
        case .transportUnavailable:
            return Problem(title: "不确定改好了没有", nextStep: "连上家里的服务器后再看一下。", detail: message)
        case .invalid:
            return Problem(title: "这样填不对", nextStep: message)
        default:
            return problem
        }
    }
}

extension PluginFailure {
    func problem(detail: String?) -> Problem {
        switch self {
        case .transportUnavailable:
            return Problem(title: "连不上家里的服务器", nextStep: "检查网络，连上后再试。", detail: detail)
        case .unauthorized:
            return Problem(title: "家里的服务器不认这台设备", nextStep: "请重新扫码配对。", detail: detail)
        case .sourceUnavailable:
            return Problem(title: "暂时取不到应用的信息", nextStep: "家里的服务器连不上应用的来源，稍后再试。", detail: detail)
        case .notFound:
            return Problem(title: "没有找到这个应用", nextStep: "刷新应用商店后再试。", detail: detail)
        case .changed:
            return Problem(title: "应用在确认之后变了", nextStep: "请重新查看、确认。", detail: detail)
        case .unsupported:
            return Problem(title: "家里的服务器不支持这个应用", nextStep: "先升级家里的服务器。", detail: detail)
        case .conflict:
            return Problem(title: "这个应用正忙", nextStep: "它已经装了，或者正在安装、升级、卸载，等那边做完。", detail: detail)
        case .invalid:
            return Problem(title: "这个应用的安装包有问题", nextStep: "请联系应用的作者。", detail: detail)
        case .deviceUnavailable:
            return Problem(title: "选择的设备不可用", nextStep: "重新查看设备后再试。", detail: detail)
        case .lanUnavailable:
            return Problem(title: "选不出局域网地址", nextStep: "在局域网设置里选好网卡或地址后再试。", detail: detail)
        case .coreUpgrading:
            return Problem(title: "家里的服务器正在升级", nextStep: "升级结束后再试。", detail: detail)
        case .egressProxyUnavailable:
            return Problem(title: "代理连不上", nextStep: "检查家里服务器上的代理，或者在出网代理设置里改地址后再试。", detail: detail)
        case .ownerProfileIncomplete:
            return Problem(title: "你的资料还没填好", nextStep: "在设置里填好后再试。", detail: detail)
        case .upstream:
            return Problem(title: "家里的服务器出错了", nextStep: "稍后再试。", detail: detail)
        case .protocol:
            return Problem(title: "App 和家里的服务器版本对不上", nextStep: "请更新 App 或家里的服务器。", detail: detail)
        }
    }
}

extension AppearanceProblem {
    var problem: Problem {
        Problem(title: "外观设置没有存上", nextStep: "检查存储空间后再试。", detail: message)
    }
}

extension ChatFailure {
    func problem(detail: String) -> Problem {
        switch self {
        case .transportUnavailable:
            return Problem(title: "连不上家里的服务器", nextStep: "检查网络，连上后再试。", detail: detail)
        case .unauthorized:
            return Problem(title: "家里的服务器不认这台设备", nextStep: "请重新扫码配对。", detail: detail)
        case .upstream:
            return Problem(title: "家里的服务器出错了", nextStep: "稍后再试。", detail: detail)
        case .notReady:
            return Problem(title: "现在还做不了", nextStep: detail)
        case .invalidInput:
            return Problem(title: "这个操作做不了", nextStep: detail)
        }
    }
}

extension ChatProblem {
    var problem: Problem { reason.problem(detail: message) }
}

extension StoreProblem {
    var problem: Problem {
        switch reason {
        case .unreachable:
            return Problem(title: "连不上应用的来源，列表可能不是最新的", detail: details)
        case .timeout:
            return Problem(title: "应用的来源太慢了，列表可能不是最新的", detail: details)
        case .rateLimited:
            return Problem(title: "请求太频繁了", nextStep: "过几分钟再试。", detail: details)
        case .invalidIndex:
            return Problem(title: "应用列表读不懂", nextStep: "稍后再试。", detail: details)
        case .indexTooNew:
            return Problem(title: "应用列表比家里的服务器新", nextStep: "先升级家里的服务器。", detail: details)
        case .upstream:
            return Problem(title: "应用的来源出了问题", nextStep: "稍后再试。", detail: details)
        case .incomplete:
            return Problem(title: "有几个应用没加载出来", detail: details)
        case .disabled:
            return Problem(title: "社区应用已关闭", detail: details)
        case .other:
            return Problem(title: "应用列表可能不完整", detail: details)
        }
    }

    /// 主人自己关掉的，重试也不会变。
    var canRetry: Bool { reason != .disabled }
}

extension Error {
    var problem: Problem {
        let detail = String(describing: self)
        guard let error = self as? CoreError else {
            return Problem(title: "出错了", nextStep: "稍后再试。", detail: detail)
        }
        switch error {
        case .TransportUnavailable:
            return Problem(title: "连不上家里的服务器", nextStep: "检查网络，连上后再试。", detail: detail)
        case .Unauthorized:
            return Problem(title: "家里的服务器不认这台设备", nextStep: "请重新扫码配对。", detail: detail)
        case .Upstream:
            return Problem(title: "家里的服务器出错了", nextStep: "稍后再试。", detail: detail)
        case let .NotReady(reason):
            return Problem(title: "现在还做不了", nextStep: reason)
        case .Storage:
            return Problem(title: "这台设备读写数据出错了", nextStep: "检查存储空间后再试。", detail: detail)
        case let .InvalidInput(reason):
            return Problem(title: "这个操作做不了", nextStep: reason)
        }
    }
}

/// 收起的技术细节。排查时才看，能选中复制。
struct ProblemDetail: View {
    let text: String
    var minimumTargetHeight: CGFloat?
    @State private var isExpanded = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
            Button {
                isExpanded.toggle()
            } label: {
                HStack(spacing: WontonTheme.spacing.s4) {
                    Text("详情")
                    WontonIcon.forward.view(size: WontonTheme.size.iconTiny)
                        .rotationEffect(isExpanded ? .degrees(90) : .zero)
                }
                .frame(minHeight: minimumTargetHeight)
                .foregroundStyle(Color(colors.textSecondary))
            }
            .buttonStyle(.plain)
            if isExpanded {
                Text(text)
                    .foregroundStyle(Color(colors.textSecondary))
                    .multilineTextAlignment(.leading)
                    .textSelection(.enabled)
            }
        }
        .wontonText(WontonTheme.typography.caption)
    }
}

/// 什么都显示不出来，整页说明。
struct ProblemView: View {
    let problem: Problem
    let retry: () -> Void
    @Environment(\.wontonColors) private var colors

    var body: some View {
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
            Button("重试", action: retry)
                .buttonStyle(WontonButtonStyle(kind: .secondary))
                .padding(.vertical, WontonTheme.spacing.s8)
            if let detail = problem.detail { ProblemDetail(text: detail) }
        }
        .multilineTextAlignment(.center)
        .padding(WontonTheme.spacing.s32)
    }
}

/// 页面还能用，只是这件事没做成。
struct ProblemNotice: View {
    let problem: Problem
    var icon: WontonIcon = .warning

    var body: some View {
        WontonNotice(icon: icon, title: problem.title, lines: problem.nextStep.map { [$0] } ?? []) {
            if let detail = problem.detail { ProblemDetail(text: detail) }
        }
    }
}

extension View {
    /// 主人点了什么没做成：说清原因就好，点“好”关掉。
    func wontonDialog(
        problem: Problem?,
        isPresented: Binding<Bool>,
        onDismiss: (() -> Void)? = nil
    ) -> some View {
        wontonDialog(
            problem?.title ?? "",
            message: problem?.nextStep,
            detail: problem?.detail,
            isPresented: isPresented,
            onDismiss: onDismiss
        )
    }
}

/// 列表还在，只是少了点什么或者不是最新的：一条轻提示，不抢列表的位置。
struct ProblemRow: View {
    let problem: Problem
    var retry: (() -> Void)?
    @Environment(\.wontonColors) private var colors

    var body: some View {
        HStack(alignment: .top, spacing: WontonTheme.spacing.s8) {
            WontonIcon.info.view(size: WontonTheme.size.iconSmall)
                .foregroundStyle(Color(colors.textSecondary))
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s4) {
                Text(problem.title)
                    .wontonText(WontonTheme.typography.supporting)
                    .foregroundStyle(Color(colors.textPrimary))
                if let nextStep = problem.nextStep {
                    Text(nextStep)
                        .wontonText(WontonTheme.typography.supporting)
                        .foregroundStyle(Color(colors.textSecondary))
                }
                if let detail = problem.detail { ProblemDetail(text: detail) }
            }
            Spacer(minLength: 0)
            if let retry {
                Button("重试", action: retry).buttonStyle(WontonSmallButtonStyle())
            }
        }
    }
}

/// 过了几秒还没好时才出现的一句话。
private struct SlowHint: View {
    @State private var isSlow = false
    @Environment(\.wontonColors) private var colors

    var body: some View {
        Text("还在加载，网络慢时要多等一会儿")
            .wontonText(WontonTheme.typography.supporting)
            .foregroundStyle(Color(colors.textSecondary))
            .multilineTextAlignment(.center)
            .opacity(isSlow ? 1 : 0)
            .task {
                try? await Task.sleep(for: .seconds(5))
                isSlow = true
            }
    }
}

/// 取一个页面的内容：转圈，慢了说一句。
struct LoadingView: View {
    var body: some View {
        VStack(spacing: WontonTheme.spacing.s12) {
            ProgressView()
            SlowHint()
        }
        .padding(WontonTheme.spacing.s32)
    }
}

/// 取列表：骨架占住列表的位置，慢了说一句。
struct ListSkeleton: View {
    let iconSize: CGFloat
    @State private var isDim = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.wontonColors) private var colors

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.s16) {
            ForEach(0 ..< 5, id: \.self) { _ in row }
            SlowHint().frame(maxWidth: .infinity)
            Spacer(minLength: 0)
        }
        .padding(WontonTheme.spacing.pageInset)
        // 键盘弹起时放不下，从上往下裁，不要盖住上面的搜索框。
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .clipped()
        .opacity(isDim ? WontonTheme.ratio.secondaryContent : 1)
        .accessibilityLabel("正在加载")
        .onAppear {
            guard !reduceMotion else { return }
            withAnimation(.easeInOut(duration: 0.9).repeatForever()) { isDim = true }
        }
    }

    private var row: some View {
        HStack(spacing: WontonTheme.spacing.s12) {
            RoundedRectangle(cornerRadius: iconSize * WontonTheme.ratio.pluginIconRadius, style: .continuous)
                .fill(Color(colors.surfaceSubtle))
                .frame(width: iconSize, height: iconSize)
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s8) {
                line.frame(maxWidth: 120)
                line
            }
        }
    }

    private var line: some View {
        Capsule().fill(Color(colors.surfaceSubtle)).frame(height: WontonTheme.spacing.s12)
    }
}
