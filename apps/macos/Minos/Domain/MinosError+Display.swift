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
        }
    }

    func userMessage(lang: Lang = .zh) -> String {
        kindMessage(kind: kind, lang: lang)
    }

    var technicalDetails: String {
        errorDescription ?? String(reflecting: self)
    }
}
