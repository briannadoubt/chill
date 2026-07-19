package dev.chill.core

import java.io.ByteArrayOutputStream
import java.net.HttpURLConnection
import java.net.URI
import java.util.zip.GZIPOutputStream
import kotlin.math.min

public data class ExportResult(val acknowledgedIds: Set<String>, val retryAfterMillis: Long? = null)
public fun interface Transport { public fun send(endpoint: String, sdkKey: String, bodies: List<String>): ExportResult }

public class OtlpHttpTransport : Transport {
    override fun send(endpoint: String, sdkKey: String, bodies: List<String>): ExportResult {
        val payload = Json.encode(
            mapOf(
                "resourceLogs" to listOf(
                    mapOf(
                        "resource" to mapOf("attributes" to listOf(mapOf("key" to "os.type", "value" to mapOf("stringValue" to "android")))),
                        "scopeLogs" to listOf(
                            mapOf(
                                "scope" to mapOf("name" to "dev.chill.android", "version" to "0.1.0"),
                                "logRecords" to bodies.map { mapOf("body" to mapOf("stringValue" to it)) },
                            ),
                        ),
                    ),
                ),
            ),
        )
        val compressed = ByteArrayOutputStream().also { output -> GZIPOutputStream(output).use { it.write(payload.toByteArray()) } }.toByteArray()
        val connection = URI(endpoint.trimEnd('/') + "/v1/logs").toURL().openConnection() as HttpURLConnection
        connection.requestMethod = "POST"; connection.connectTimeout = 10_000; connection.readTimeout = 20_000; connection.doOutput = true
        connection.setRequestProperty("Authorization", "Bearer $sdkKey"); connection.setRequestProperty("Content-Type", "application/json"); connection.setRequestProperty("Content-Encoding", "gzip"); connection.setRequestProperty("X-Chill-Schema-Version", "1.0.0")
        connection.outputStream.use { it.write(compressed) }; val status = connection.responseCode
        return when (status) { in 200..299 -> ExportResult(emptySet()); 429, 503 -> throw RetryableExportException(status); else -> throw PermanentExportException(status) }
    }
}
public class RetryableExportException(status: Int) : Exception("retryable Chill export HTTP $status")
public class PermanentExportException(status: Int) : Exception("permanent Chill export HTTP $status")

public class OfflineExporter(private val runtime: ChillRuntime, private val transport: Transport = OtlpHttpTransport(), private val sleep: (Long) -> Unit = Thread::sleep, private val jitter: () -> Long = { kotlin.random.Random.nextLong(0, 125) }) {
    public fun flush(maxAttempts: Int = 4): Int { val batch = runtime.pending(); if (batch.isEmpty()) return 0; var delay = 250L; repeat(maxAttempts) { attempt -> try { transport.send(runtime.configuration.endpoint, runtime.configuration.sdkKey, batch.map { it.body }); runtime.acknowledge(batch.mapTo(mutableSetOf()) { it.id }); return batch.size } catch (error: RetryableExportException) { if (attempt == maxAttempts - 1) throw error; sleep(delay + jitter()); delay = min(delay * 2, 8_000L) } }; return 0 }
}
