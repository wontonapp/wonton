import WontonClient

extension ModelProviderProblem {
    var onboardingProblem: Problem {
        let title: String
        let nextStep: String
        switch reason {
        case .keyRejected:
            title = "服务商不接受这个 key"
            nextStep = "key 没有保存，改好后再试。"
        case .providerUnreachable:
            title = "连不上服务商"
            nextStep = "key 已保存，可以继续，稍后再测试。"
        case .gatewayNotReady:
            title = "模型网关还没准备好"
            nextStep = "稍后重试，也可以先跳过。"
        case .transportUnavailable:
            title = "连不上家里的服务器"
            nextStep = "检查网络，连上后再试。"
        case .unauthorized:
            title = "家里的服务器不认这台设备"
            nextStep = "请重新扫码配对。"
        case .upstream:
            title = "家里的服务器出错了"
            nextStep = "稍后再试。"
        case .protocol:
            title = "App 和家里的服务器版本对不上"
            nextStep = "请更新 App 或家里的服务器。"
        }
        return Problem(title: title, nextStep: nextStep, detail: message)
    }
}

extension StoreProblem {
    var onboardingProblem: Problem {
        let title: String
        switch reason {
        case .unreachable:
            title = "取不到推荐清单"
        case .timeout:
            title = "推荐清单加载太慢了"
        case .rateLimited:
            title = "请求太频繁了"
        case .invalidIndex:
            title = "读不懂推荐清单"
        case .indexTooNew:
            title = "推荐清单需要更新的服务器"
        case .upstream:
            title = "推荐清单的来源出错了"
        case .incomplete:
            title = "部分推荐应用没加载出来"
        case .disabled:
            title = "推荐来源已关闭"
        case .other:
            title = "推荐清单不可用"
        }
        return Problem(title: title, nextStep: "可以跳过，之后在应用商店安装。", detail: details)
    }
}
