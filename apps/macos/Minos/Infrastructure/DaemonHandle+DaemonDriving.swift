import Foundation

extension Subscription: SubscriptionHandle {}

extension DaemonHandle: DaemonDriving {
    func subscribeObserver(_ observer: ConnectionStateObserver) -> any SubscriptionHandle {
        subscribe(observer: observer)
    }
}
