package dev.chill.core

import java.security.SecureRandom
import java.time.Instant
import java.util.UUID
import java.util.concurrent.atomic.AtomicLong

public enum class Consent { UNKNOWN, DENIED, GRANTED }
public enum class Kind { SESSION, JOURNEY, PAGE, IMPRESSION, ACTION, ACTIVITY, EVENT, REPLAY }
public enum class Operation { INSTANT, START, UPDATE, END }
public data class PageIdentity(val instanceId: String, val path: List<String>, val pathInstanceIds: List<String>)
public data class TraceContext(val traceId: String, val spanId: String, val sampled: Boolean = true)
public data class Clock(val wallUnixNano: String, val monotonicNano: String, val sequenceNumber: Long)
public data class BehaviorRecord(
    val recordId: String,
    val subjectId: String,
    val kind: Kind,
    val operation: Operation,
    val name: String,
    val clock: Clock,
    val sessionId: String,
    val replayId: String,
    val page: PageIdentity?,
    val annotations: Map<String, Any>,
    val consent: Consent,
    val policyVersion: String,
    val trace: TraceContext?,
    val payload: Map<String, Any>,
)

internal object Identity {
    private val random = SecureRandom()
    fun uuidV7(nowMillis: Long = System.currentTimeMillis()): String {
        val bytes = ByteArray(16).also(random::nextBytes)
        var time = nowMillis
        for (index in 5 downTo 0) { bytes[index] = (time and 0xff).toByte(); time = time ushr 8 }
        bytes[6] = ((bytes[6].toInt() and 0x0f) or 0x70).toByte(); bytes[8] = ((bytes[8].toInt() and 0x3f) or 0x80).toByte()
        val high = bytes.take(8).fold(0L) { value, byte -> (value shl 8) or (byte.toLong() and 0xff) }
        val low = bytes.drop(8).fold(0L) { value, byte -> (value shl 8) or (byte.toLong() and 0xff) }
        return UUID(high, low).toString()
    }
}

public class ChillClock(private val wallMillis: () -> Long = System::currentTimeMillis, private val monotonicNanos: () -> Long = System::nanoTime) {
    private val sequence = AtomicLong()
    public fun next(): Clock = Clock("${wallMillis()}000000", monotonicNanos().toString(), sequence.incrementAndGet())
}
