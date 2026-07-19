package dev.chill.core

import java.util.concurrent.ConcurrentHashMap
import kotlin.random.Random

public data class ChillConfiguration(
    val endpoint: String,
    val sdkKey: String,
    val policyVersion: String,
    val consent: Consent = Consent.UNKNOWN,
    val sampleRate: Double = 1.0,
    val replaySampleRate: Double = 0.0,
    val trustedTraceOrigins: Set<String> = emptySet(),
)

public class ChillRuntime(
    public val configuration: ChillConfiguration,
    private val queue: RecordQueue = MemoryRecordQueue(),
    private val clock: ChillClock = ChillClock(),
    random: Random = Random.Default,
) {
    public val sessionId: String = Identity.uuidV7(); public val replayId: String = Identity.uuidV7()
    public val diagnostics: MutableList<String> = java.util.Collections.synchronizedList(mutableListOf())
    private val sampled = random.nextDouble() < configuration.sampleRate; private val replaySampled = random.nextDouble() < configuration.replaySampleRate
    private val activations = ConcurrentHashMap.newKeySet<String>(); private var consent = configuration.consent; private var context = AnnotationContext.empty(); private var page: PageIdentity? = null; private var trace: TraceContext? = null
    init { require(configuration.sampleRate in 0.0..1.0 && configuration.replaySampleRate in 0.0..1.0); if (canCapture()) emit(Kind.SESSION, Operation.START, "app.session") }

    public fun setConsent(value: Consent) { val previous = consent; consent = value; if (value == Consent.DENIED) queue.clear(); if (previous != Consent.GRANTED && canCapture()) emit(Kind.SESSION, Operation.START, "app.session") }
    public fun setContext(value: AnnotationContext) { context = value; diagnostics += value.diagnostics.map { "${it.code}:${it.key}" } }
    public fun setTrace(value: TraceContext?) { trace = value }
    public fun currentTrace(): TraceContext? = trace
    public fun currentPage(): PageIdentity? = page
    public fun shouldReplay(): Boolean = canCapture() && replaySampled
    public fun startPage(segment: String, parent: PageIdentity? = page): PageIdentity { require(SEMANTIC.matches(segment)); page?.let { emit(Kind.PAGE, Operation.END, it.path.last(), pageOverride = it) }; val id = Identity.uuidV7(); return PageIdentity(id, (parent?.path ?: emptyList()) + segment, (parent?.pathInstanceIds ?: emptyList()) + id).also { page = it; emit(Kind.PAGE, Operation.START, segment, pageOverride = it) } }
    public fun action(name: String, payload: Map<String, Any> = emptyMap()): Unit = emit(Kind.ACTION, Operation.INSTANT, name, payload)
    public fun observeAction(name: String, activationId: String, surfaceId: String, payload: Map<String, Any> = emptyMap()): Boolean { val key = "$surfaceId\u0000$activationId"; if (!activations.add(key)) { diagnostics += "action.duplicate_observation"; return false }; if (activations.size > 512) activations.firstOrNull()?.let(activations::remove); action(name, payload + mapOf("native_activation_id" to activationId, "surface_id" to surfaceId)); return true }
    public fun event(name: String, payload: Map<String, Any> = emptyMap()): Unit = emit(Kind.EVENT, Operation.INSTANT, name, payload)
    public fun impression(name: String, payload: Map<String, Any> = emptyMap()): Unit = emit(Kind.IMPRESSION, Operation.INSTANT, name, payload)
    public fun replay(payload: Map<String, Any>): Unit = emit(Kind.REPLAY, Operation.INSTANT, "session.replay", payload)
    public fun <T> activity(name: String, work: () -> T): T { val subject = Identity.uuidV7(); emit(Kind.ACTIVITY, Operation.START, name, subjectId = subject); return try { work().also { emit(Kind.ACTIVITY, Operation.END, name, mapOf("outcome" to "success"), subjectId = subject) } } catch (error: Throwable) { emit(Kind.ACTIVITY, Operation.END, name, mapOf("outcome" to if (error is java.util.concurrent.CancellationException) "cancelled" else "failure", "error_type" to error.javaClass.simpleName), subjectId = subject); throw error } }
    public fun pending(limit: Int = 200): List<QueuedRecord> = queue.peek(limit)
    public fun acknowledge(ids: Set<String>): Unit = queue.acknowledge(ids)
    public fun stop(): Unit = emit(Kind.SESSION, Operation.END, "app.session")

    @PublishedApi internal fun emit(kind: Kind, operation: Operation, name: String, payload: Map<String, Any> = emptyMap(), pageOverride: PageIdentity? = page, subjectId: String = Identity.uuidV7()) {
        if (!canCapture()) return; if (!SEMANTIC.matches(name)) { diagnostics += "invalid_name:$name"; return }
        val record = BehaviorRecord(Identity.uuidV7(), subjectId, kind, operation, name, clock.next(), sessionId, replayId, pageOverride, context.values, consent, configuration.policyVersion, trace, payload)
        val priority = when (kind) { Kind.REPLAY -> 0; Kind.IMPRESSION -> 1; else -> 2 }; queue.enqueue(QueuedRecord(record.recordId, kind, priority, Json.record(record)))
    }
    private fun canCapture(): Boolean = sampled && consent == Consent.GRANTED
    private companion object { val SEMANTIC = Regex("^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$") }
}
