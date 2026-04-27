import Foundation

extension MinosError {
    var kind: ErrorKind {
        switch self {
        case .BindFailed:
            return .bindFailed
        case .ConnectFailed:
            return .connectFailed
        case .Disconnected:
            return .disconnected
        case .PairingTokenInvalid:
            return .pairingTokenInvalid
        case .PairingStateMismatch:
            return .pairingStateMismatch
        case .DeviceNotTrusted:
            return .deviceNotTrusted
        case .StoreIo:
            return .storeIo
        case .StoreCorrupt:
            return .storeCorrupt
        case .CliProbeTimeout:
            return .cliProbeTimeout
        case .CliProbeFailed:
            return .cliProbeFailed
        case .RpcCallFailed:
            return .rpcCallFailed
        case .Unauthorized:
            return .unauthorized
        case .ConnectionStateMismatch:
            return .connectionStateMismatch
        case .EnvelopeVersionUnsupported:
            return .envelopeVersionUnsupported
        case .PeerOffline:
            return .peerOffline
        case .BackendInternal:
            return .backendInternal
        case .CfAuthFailed:
            return .cfAuthFailed
        case .CodexSpawnFailed:
            return .codexSpawnFailed
        case .CodexConnectFailed:
            return .codexConnectFailed
        case .CodexProtocolError:
            return .codexProtocolError
        case .AgentAlreadyRunning:
            return .agentAlreadyRunning
        case .AgentNotRunning:
            return .agentNotRunning
        case .AgentNotSupported:
            return .agentNotSupported
        case .AgentSessionIdMismatch:
            return .agentSessionIdMismatch
        case .CfAccessMisconfigured:
            return .cfAccessMisconfigured
        case .IngestSeqConflict:
            return .ingestSeqConflict
        case .ThreadNotFound:
            return .threadNotFound
        case .TranslationNotImplemented:
            return .translationNotImplemented
        case .TranslationFailed:
            return .translationFailed
        case .PairingQrVersionUnsupported:
            return .pairingQrVersionUnsupported
        case .Timeout:
            return .timeout
        case .NotConnected:
            return .notConnected
        case .RequestDropped:
            return .requestDropped
        case .AuthRefreshFailed:
            return .authRefreshFailed
        case .EmailTaken:
            return .emailTaken
        case .WeakPassword:
            return .weakPassword
        case .RateLimited:
            return .rateLimited
        case .InvalidCredentials:
            return .invalidCredentials
        case .AgentStartFailed:
            return .agentStartFailed
        case .PairingTokenExpired:
            return .pairingTokenExpired
        }
    }

    func userMessage(lang: Lang = .zh) -> String {
        kindMessage(kind: kind, lang: lang)
    }

    var technicalDetails: String {
        errorDescription ?? String(reflecting: self)
    }
}
