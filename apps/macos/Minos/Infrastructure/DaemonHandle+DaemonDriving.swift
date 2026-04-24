import Foundation

extension Subscription: SubscriptionHandle {}

extension DaemonHandle: DaemonDriving {
    func startAgent(_ req: StartAgentRequest) async throws -> StartAgentResponse {
        try await startAgent(req: req)
    }

    func sendUserMessage(_ req: SendUserMessageRequest) async throws {
        try await sendUserMessage(req: req)
    }

    func subscribeObserver(_ observer: ConnectionStateObserver) -> any SubscriptionHandle {
        subscribe(observer: observer)
    }

    func subscribeAgentState(_ observer: AgentStateObserver) -> any SubscriptionHandle {
        subscribeAgentState(observer: observer)
    }
}
