package dev.chill.core

import java.io.File
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.concurrent.locks.ReentrantLock
import kotlin.concurrent.withLock

public data class QueuedRecord(val id: String, val kind: Kind, val priority: Int, val body: String) { public val bytes: Int get() = body.toByteArray(StandardCharsets.UTF_8).size }
public interface RecordQueue { public fun enqueue(record: QueuedRecord); public fun peek(limit: Int): List<QueuedRecord>; public fun acknowledge(ids: Set<String>); public fun clear(); public val size: Int }

public class FileRecordQueue(private val file: File, private val maximumBytes: Int = 8 * 1024 * 1024) : RecordQueue {
    private val lock = ReentrantLock(); private var records = load()
    override val size: Int get() = lock.withLock { records.size }
    override fun enqueue(record: QueuedRecord): Unit = lock.withLock { records += record; enforceCapacity(); persist() }
    override fun peek(limit: Int): List<QueuedRecord> = lock.withLock { records.take(limit) }
    override fun acknowledge(ids: Set<String>): Unit = lock.withLock { records = records.filterNot { it.id in ids }; persist() }
    override fun clear(): Unit = lock.withLock { records = emptyList(); persist() }
    private fun enforceCapacity() { while (records.sumOf { it.bytes } > maximumBytes && records.isNotEmpty()) { val victim = records.withIndex().minWithOrNull(compareBy<IndexedValue<QueuedRecord>> { it.value.priority }.thenBy { it.index })!!; records = records.filterIndexed { index, _ -> index != victim.index } } }
    private fun load(): List<QueuedRecord> = try { if (!file.exists()) emptyList() else file.readLines().mapNotNull { line -> val fields = line.split('\t', limit = 4); if (fields.size == 4) QueuedRecord(fields[0], Kind.valueOf(fields[1]), fields[2].toInt(), String(java.util.Base64.getDecoder().decode(fields[3]), StandardCharsets.UTF_8)) else null } } catch (_: Exception) { emptyList() }
    private fun persist() { file.parentFile?.mkdirs(); val temporary = File(file.parentFile, "${file.name}.tmp"); temporary.writeText(records.joinToString("\n") { "${it.id}\t${it.kind.name}\t${it.priority}\t${java.util.Base64.getEncoder().encodeToString(it.body.toByteArray(StandardCharsets.UTF_8))}" }); try { Files.move(temporary.toPath(), file.toPath(), StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE) } catch (_: Exception) { Files.move(temporary.toPath(), file.toPath(), StandardCopyOption.REPLACE_EXISTING) } }
}

public class MemoryRecordQueue(private val maximumRecords: Int = 2_000) : RecordQueue {
    private val records = mutableListOf<QueuedRecord>()
    override val size: Int get() = synchronized(records) { records.size }
    override fun enqueue(record: QueuedRecord): Unit = synchronized(records) { records += record; while (records.size > maximumRecords) records.removeAt(records.indexOfFirst { it.kind == Kind.REPLAY }.takeIf { it >= 0 } ?: 0) }
    override fun peek(limit: Int): List<QueuedRecord> = synchronized(records) { records.take(limit) }
    override fun acknowledge(ids: Set<String>): Unit = synchronized(records) { records.removeAll { it.id in ids } }
    override fun clear(): Unit = synchronized(records) { records.clear() }
}
