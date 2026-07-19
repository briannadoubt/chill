package dev.chill.core

import java.net.URI
import java.security.SecureRandom

public object TracePropagation {
    private val random = SecureRandom()
    public fun parse(value: String?): TraceContext? { val match = Regex("^00-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$").matchEntire(value ?: return null) ?: return null; val (trace, span, flags) = match.destructured; if (trace.all { it == '0' } || span.all { it == '0' }) return null; return TraceContext(trace, span, flags.toInt(16) and 1 == 1) }
    public fun child(parent: TraceContext?): TraceContext = TraceContext(parent?.traceId ?: hex(16), hex(8), parent?.sampled ?: true)
    public fun header(context: TraceContext): String = "00-${context.traceId}-${context.spanId}-${if (context.sampled) "01" else "00"}"
    public fun headers(url: String, trustedOrigins: Set<String>, parent: TraceContext?): Map<String, String> { val uri = URI(url); val origin = "${uri.scheme}://${uri.host}${if (uri.port == -1) "" else ":${uri.port}"}"; return if (origin in trustedOrigins) mapOf("traceparent" to header(child(parent))) else emptyMap() }
    private fun hex(size: Int): String = ByteArray(size).also(random::nextBytes).joinToString("") { "%02x".format(it) }
}
