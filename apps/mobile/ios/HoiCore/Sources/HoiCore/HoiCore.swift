import Combine
import Foundation
import SwiftUI

/// The app's handle on hoi: opens the node, keeps `me`, `communities` and `net` fresh for
/// SwiftUI, and follows the app's lifecycle. Everything else is the generated `HoiNode` API,
/// called through `run` so the main thread never waits on storage, mining or the network.
///
/// ```swift
/// @main struct HoiApp: App {
///     @Environment(\.scenePhase) private var phase
///     @State private var hoi: Hoi?
///     var body: some Scene {
///         WindowGroup {
///             Group { if let hoi { RootView().environmentObject(hoi) } else { ProgressView() } }
///                 .task { hoi = try? await Hoi.open() }
///         }
///         .onChange(of: phase) { hoi?.scenePhaseChanged($0) }
///     }
/// }
/// ```
@MainActor
public final class Hoi: ObservableObject {
    /// The node. Its methods block: call them through `run`.
    public nonisolated let node: HoiNode

    @Published public private(set) var me: Me?
    @Published public private(set) var communities: [CommunitySummary] = []
    /// The relay pill: `net.label` reads "via mira's computer", "looking for your home node", …
    @Published public private(set) var net: NetStatus
    /// When this phone last reached its home node: "last synced 4 min ago".
    @Published public private(set) var lastSynced: Date?

    /// Every event from the core. Screens reload on `.changed`, animate the work bar on
    /// `.stampStarted`/`.stampDone` and toast `.notice`.
    public let events = PassthroughSubject<HoiEvent, Never>()

    private var listening: Task<Void, Never>?
    private var refreshing = false
    private var refreshAgain = false

    private init(node: HoiNode, snapshot: Snapshot) {
        self.node = node
        self.net = snapshot.net
        apply(snapshot)
    }

    /// Opens the node in Application Support/hoi with keys in the Keychain. Replays and checks
    /// everything stored, so it runs off the main thread. Keep the result for the app's lifetime.
    public static func open(directory: URL? = nil, keyStore: KeyStore = KeychainKeyStore()) async throws -> Hoi {
        let dir = try directory ?? defaultDirectory()
        let (stream, continuation) = AsyncStream.makeStream(of: String.self, bufferingPolicy: .bufferingNewest(512))
        let relay = EventRelay(continuation)
        let (node, snapshot) = try await Task.detached(priority: .userInitiated) {
            let node = try HoiNode.open(dataDir: dir.path, keystore: keyStore, listener: relay)
            return (node, Snapshot(node))
        }.value
        let hoi = Hoi(node: node, snapshot: snapshot)
        hoi.listen(stream)
        return hoi
    }

    /// Application Support/hoi, kept out of backups: keys never leave this phone, and records
    /// restored without them would make an identity that can't sign.
    public nonisolated static func defaultDirectory() throws -> URL {
        var dir = try FileManager.default
            .url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            .appendingPathComponent("hoi", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? dir.setResourceValues(values)
        return dir
    }

    /// Runs `work` on a background thread. Every `HoiNode` method blocks while it reads storage,
    /// validates, mines a stamp or writes: `try await hoi.run { try $0.createPost(draft: d) }`.
    public nonisolated func run<T: Sendable>(_ work: @escaping @Sendable (HoiNode) throws -> T) async throws -> T {
        let node = self.node
        return try await Task.detached(priority: .userInitiated) { try work(node) }.value
    }

    // MARK: lifecycle

    /// Phones get no background sockets (TECHNICAL.md §4): the home link runs while the app is
    /// active and closes when it leaves. Call from `.onChange(of: scenePhase)`.
    public func scenePhaseChanged(_ phase: ScenePhase) {
        switch phase {
        case .active:
            Task {
                _ = try? await run { node in
                    _ = node.startHomeLink()
                    _ = node.sweep()
                }
                await refresh()
            }
        case .background:
            Task { _ = try? await run { $0.stopHomeLink() } }
        default:
            break
        }
    }

    /// One background sync for a `BGAppRefreshTask`: connects, waits until the home node goes
    /// quiet, disconnects. Returns whether the home node was reached.
    public nonisolated func syncBurst(maxSeconds: UInt32 = 20) async -> Bool {
        let reached = (try? await run { $0.syncBurst(maxSeconds: maxSeconds) }) ?? false
        await refresh()
        return reached
    }

    /// Pairs with a home node from a scanned `hoi://pair?…` QR code and connects right away.
    public func pair(_ link: String) async throws {
        _ = try await run { node in
            let status = try node.pair(link: link)
            _ = node.startHomeLink()
            return status
        }
        await refresh()
    }

    // MARK: state

    /// Reloads `me`, `communities`, `net` and `lastSynced`. Happens on its own after `changed`
    /// and `net` events; bursts of events coalesce into one reload.
    public func refresh() async {
        if refreshing {
            refreshAgain = true
            return
        }
        refreshing = true
        defer { refreshing = false }
        repeat {
            refreshAgain = false
            guard let snapshot = try? await run({ Snapshot($0) }) else { return }
            apply(snapshot)
        } while refreshAgain
    }

    private func apply(_ s: Snapshot) {
        me = s.me
        communities = s.communities
        net = s.net
        lastSynced = s.lastSyncedAt.map { Date(timeIntervalSince1970: TimeInterval($0)) }
    }

    private func listen(_ stream: AsyncStream<String>) {
        listening = Task { [weak self] in
            for await json in stream {
                guard let self else { return }
                guard let event = HoiEvent(json: json) else { continue }
                switch event {
                case .changed, .net: Task { await self.refresh() }
                default: break
                }
                self.events.send(event)
            }
        }
    }
}

/// What the core reports, decoded from its JSON events (packages/core-api `CoreEvent`).
public enum HoiEvent: Sendable, Equatable {
    /// Records arrived, expired or were removed in these communities: reload what's on screen.
    case changed(communities: [String])
    /// Worth a toast, e.g. "Bounced f30a·c812: too many bad records. …".
    case notice(String)
    /// Proof-of-work started for something you're signing: fill the work bar over `estimateMs`.
    case stampStarted(bits: Int, estimateMs: Int)
    /// The stamp is done: land the "SIGNED · POW n" stamp.
    case stampDone(bits: Int, tookMs: Int)
    /// The home-node link changed; `Hoi.net` follows.
    case net

    init?(json: String) {
        guard let data = json.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let type = o["type"] as? String
        else { return nil }
        switch type {
        case "changed":
            self = .changed(communities: o["communities"] as? [String] ?? [])
        case "notice":
            self = .notice(o["text"] as? String ?? "")
        case "stamp":
            let bits = o["bits"] as? Int ?? 0
            if o["phase"] as? String == "done" {
                self = .stampDone(bits: bits, tookMs: o["tookMs"] as? Int ?? 0)
            } else {
                self = .stampStarted(bits: bits, estimateMs: o["estimateMs"] as? Int ?? 0)
            }
        case "net":
            self = .net
        default:
            return nil
        }
    }
}

/// Hands events from the core's threads to the main actor without blocking them.
private final class EventRelay: EventListener {
    let continuation: AsyncStream<String>.Continuation

    init(_ continuation: AsyncStream<String>.Continuation) {
        self.continuation = continuation
    }

    func onEvent(json: String) {
        continuation.yield(json)
    }
}

private struct Snapshot: Sendable {
    let me: Me?
    let communities: [CommunitySummary]
    let net: NetStatus
    let lastSyncedAt: UInt64?

    init(_ node: HoiNode) {
        me = node.me()
        communities = node.communities()
        net = node.netStatus()
        lastSyncedAt = node.lastSyncedAt()
    }
}
