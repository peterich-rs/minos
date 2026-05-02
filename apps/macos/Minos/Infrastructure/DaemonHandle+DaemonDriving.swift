import Foundation

extension Subscription: SubscriptionHandle {}

extension DaemonHandle: DaemonDriving {
    // The UniFFI-generated methods take `observer:` and `req:` argument
    // labels because they originate from Rust impl blocks; the protocol
    // uses positional names for ergonomics. These thin shims bridge the
    // two without losing the trailing-closure-style call sites in
    // SwiftUI views.
    func startAgent(_ req: StartAgentRequest) async throws -> StartAgentResponse {
        try await startAgent(req: req)
    }

    func sendUserMessage(_ req: SendUserMessageRequest) async throws {
        try await sendUserMessage(req: req)
    }

    func interruptThread(_ req: InterruptThreadRequest) async throws {
        try await interruptThread(req: req)
    }

    func closeThread(_ req: CloseThreadRequest) async throws {
        try await closeThread(req: req)
    }

    func subscribeRelayLink(_ observer: RelayLinkStateObserver) -> any SubscriptionHandle {
        subscribeRelayLink(observer: observer)
    }

    func subscribePeer(_ observer: PeerStateObserver) -> any SubscriptionHandle {
        subscribePeer(observer: observer)
    }

    func subscribeAgentState(_ observer: AgentStateObserver) -> any SubscriptionHandle {
        subscribeAgentState(observer: observer)
    }
}
