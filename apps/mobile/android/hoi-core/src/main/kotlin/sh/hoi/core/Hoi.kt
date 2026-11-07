package sh.hoi.core

import android.content.Context
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import uniffi.hoi_core.CommunitySummary
import uniffi.hoi_core.Me
import uniffi.hoi_core.NetStatus
import uniffi.hoi_ffi.EventListener
import uniffi.hoi_ffi.HoiException
import uniffi.hoi_ffi.HoiNode
import uniffi.hoi_ffi.KeyStore

/**
 * The app's handle on hoi: opens the node, keeps [me], [communities] and [net] fresh as
 * StateFlows for Compose, and follows the app's lifecycle. Everything else is the generated
 * [HoiNode] API, called through [run] so the main thread never waits on storage, mining or the
 * network.
 *
 * Keep one per process (hold it in your Application):
 * ```
 * class HoiApp : Application() {
 *     val hoi = CoroutineScope(Dispatchers.Default).async { Hoi.open(this@HoiApp) }
 *     override fun onCreate() {
 *         super.onCreate()
 *         ProcessLifecycleOwner.get().lifecycle.addObserver(LifecycleEventObserver { _, e ->
 *             if (hoi.isCompleted) when (e) {
 *                 Lifecycle.Event.ON_START -> hoi.getCompleted().foreground()
 *                 Lifecycle.Event.ON_STOP -> hoi.getCompleted().background()
 *                 else -> {}
 *             }
 *         })
 *     }
 * }
 * ```
 */
class Hoi private constructor(
    /** The node. Its methods block: call them through [run]. */
    val node: HoiNode,
    raw: SharedFlow<String>,
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private val _me = MutableStateFlow<Me?>(null)
    val me: StateFlow<Me?> = _me.asStateFlow()

    private val _communities = MutableStateFlow<List<CommunitySummary>>(emptyList())
    val communities: StateFlow<List<CommunitySummary>> = _communities.asStateFlow()

    /** The relay pill: `net.label` reads "via mira's computer", "looking for your home node", … */
    private val _net = MutableStateFlow(node.netStatus())
    val net: StateFlow<NetStatus> = _net.asStateFlow()

    /** When this phone last reached its home node (Unix seconds): "last synced 4 min ago". */
    private val _lastSyncedAt = MutableStateFlow<Long?>(null)
    val lastSyncedAt: StateFlow<Long?> = _lastSyncedAt.asStateFlow()

    private val _events = MutableSharedFlow<HoiEvent>(extraBufferCapacity = 64, onBufferOverflow = BufferOverflow.DROP_OLDEST)

    /** Every event: reload on [HoiEvent.Changed], drive the work bar on the stamps, toast notices. */
    val events: SharedFlow<HoiEvent> = _events.asSharedFlow()

    // One pending reload at most: bursts of events coalesce.
    private val reloads = MutableSharedFlow<Unit>(extraBufferCapacity = 1, onBufferOverflow = BufferOverflow.DROP_OLDEST)

    init {
        scope.launch { reloads.collect { refresh() } }
        scope.launch {
            raw.collect { json ->
                val event = HoiEvent.parse(json) ?: return@collect
                if (event is HoiEvent.Changed || event is HoiEvent.Net) reloads.tryEmit(Unit)
                _events.emit(event)
            }
        }
    }

    /** Runs [work] on [Dispatchers.IO]: every [HoiNode] method blocks while it works. */
    suspend fun <T> run(work: (HoiNode) -> T): T = withContext(Dispatchers.IO) { work(node) }

    /** Reloads [me], [communities], [net] and [lastSyncedAt]. Happens on its own after events. */
    suspend fun refresh() = withContext(Dispatchers.IO) {
        _me.value = node.me()
        _communities.value = node.communities()
        _net.value = node.netStatus()
        _lastSyncedAt.value = node.lastSyncedAt()?.toLong()
    }

    /**
     * Phones get no background sockets (TECHNICAL.md §4): the home link runs while the app is
     * visible. Call on `ON_START` of `ProcessLifecycleOwner`.
     */
    fun foreground() {
        scope.launch {
            node.startHomeLink()
            node.sweep()
            refresh()
        }
    }

    /** Closes the home link. Call on `ON_STOP` of `ProcessLifecycleOwner`. */
    fun background() {
        scope.launch { node.stopHomeLink() }
    }

    /**
     * One background sync for a WorkManager worker: connects, waits until the home node goes
     * quiet, disconnects. Returns whether the home node was reached.
     */
    suspend fun syncBurst(maxSeconds: UInt = 20u): Boolean {
        val reached = run { it.syncBurst(maxSeconds) }
        refresh()
        return reached
    }

    /** Pairs with a home node from a scanned `hoi://pair?…` QR code and connects right away. */
    suspend fun pair(link: String) {
        run {
            it.pair(link)
            it.startHomeLink()
        }
        refresh()
    }

    companion object {
        /**
         * Opens the node in `filesDir/hoi`, keys in the Android Keystore. Replays and checks
         * everything stored, so it runs on [Dispatchers.IO].
         */
        suspend fun open(context: Context, keyStore: KeyStore = KeystoreKeyStore(context.applicationContext)): Hoi =
            withContext(Dispatchers.IO) {
                val raw = MutableSharedFlow<String>(extraBufferCapacity = 512, onBufferOverflow = BufferOverflow.DROP_OLDEST)
                val listener = object : EventListener {
                    // Called on the core's threads: hand over, never block.
                    override fun onEvent(json: String) {
                        raw.tryEmit(json)
                    }
                }
                val dir = File(context.applicationContext.filesDir, "hoi")
                val node = HoiNode.open(dir.path, keyStore, listener)
                Hoi(node, raw).also { it.refresh() }
            }
    }
}

/** What the core reports, decoded from its JSON events (packages/core-api `CoreEvent`). */
sealed interface HoiEvent {
    /** Records arrived, expired or were removed in these communities: reload what's on screen. */
    data class Changed(val communities: List<String>) : HoiEvent

    /** Worth a toast, e.g. "Bounced f30a·c812: too many bad records. …". */
    data class Notice(val text: String) : HoiEvent

    /** Proof-of-work started for something you're signing: fill the work bar over [estimateMs]. */
    data class StampStarted(val bits: Int, val estimateMs: Long) : HoiEvent

    /** The stamp is done: land the "SIGNED · POW n" stamp. */
    data class StampDone(val bits: Int, val tookMs: Long) : HoiEvent

    /** The home-node link changed; [Hoi.net] follows. */
    data object Net : HoiEvent

    companion object {
        fun parse(json: String): HoiEvent? = runCatching {
            val o = JSONObject(json)
            when (o.getString("type")) {
                "changed" -> {
                    val a = o.getJSONArray("communities")
                    Changed(List(a.length()) { a.getString(it) })
                }
                "notice" -> Notice(o.getString("text"))
                "stamp" ->
                    if (o.optString("phase") == "done") {
                        StampDone(o.optInt("bits"), o.optLong("tookMs"))
                    } else {
                        StampStarted(o.optInt("bits"), o.optLong("estimateMs"))
                    }
                "net" -> Net
                else -> null
            }
        }.getOrNull()
    }
}

/** The sentence to show for a failed call: the core's own words when it has them. */
val Throwable.hoiMessage: String
    get() = (this as? HoiException.Api)?.reason ?: message ?: "Something went wrong."

/** The core's error code (`not-found`, `rate-limited`, …) for a failed call, if it has one. */
val Throwable.hoiCode: String?
    get() = (this as? HoiException.Api)?.code
