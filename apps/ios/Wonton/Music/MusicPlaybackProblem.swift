import WontonClient

extension MediaError {
    var problem: Problem {
        switch self {
        case let .Failed(failure): failure.problem
        }
    }
}

extension MediaFailure {
    /// 放歌没成：类别由核心给，文字按类别写在这里，原话收进详情。
    var problem: Problem {
        switch code {
        case .transportUnavailable:
            Problem(
                title: String(localized: .Problem2.problemTransportUnavailableTitle),
                nextStep: String(localized: .Problem3.problemTransportUnavailableNextStep),
                detail: message
            )
        case .pluginUnavailable:
            Problem(
                title: String(localized: .Music1.musicProblemNotReadyTitle),
                nextStep: String(localized: .Music0.musicProblemNotReadyNextStep),
                detail: message
            )
        case .authenticationRequired:
            Problem(
                title: String(localized: .Music0.musicProblemNotSignedInTitle),
                nextStep: String(localized: .Music0.musicProblemNotSignedInNextStep),
                detail: message
            )
        case .permissionDenied:
            Problem(
                title: String(localized: .Music1.musicProblemForbiddenTitle),
                nextStep: String(localized: .Music1.musicProblemForbiddenNextStep),
                detail: message
            )
        case .mediaGone:
            Problem(
                title: String(localized: .Music0.musicProblemNotFoundTitle),
                nextStep: String(localized: .Music0.musicProblemNotFoundNextStep),
                detail: message
            )
        case .unsupportedByDevice:
            Problem(
                title: String(localized: .Music1.musicPlaybackUnsupportedTitle),
                nextStep: String(localized: .Music1.musicPlaybackUnsupportedNextStep),
                detail: message
            )
        case .upstreamError, .rateLimited:
            Problem(
                title: String(localized: .Music1.musicProblemUpstreamTitle),
                nextStep: String(localized: .Problem2.problemUpstreamNextStep),
                detail: message
            )
        case .protocol:
            Problem(
                title: String(localized: .Music0.musicProblemProtocolTitle),
                nextStep: String(localized: .Problem0.problemPluginProtocolNextStep),
                detail: message
            )
        case .localSourceUnavailable, .localPermissionDenied, .localStorageFull, .localIo, .serverStorageFull,
             .suspendedBySystem, .sourceRequired, .staleRevision, .other:
            Problem(
                title: String(localized: .Music0.musicPlaybackFailedTitle),
                nextStep: String(localized: .Music0.musicPlaybackFailedNextStep),
                detail: message
            )
        }
    }
}
