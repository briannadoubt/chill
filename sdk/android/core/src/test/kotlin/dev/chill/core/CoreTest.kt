package dev.chill.core

import java.io.File
import kotlin.random.Random
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class CoreTest {
    @Test fun outerFirstContextDiagnosesCollisions() { val tier = AnnotationKey<String>("account.tier"); val merged = AnnotationContext.empty().with(tier, "enterprise").mergeDescendant(AnnotationContext.empty().with(tier, "free")); assertEquals("enterprise", merged.values[tier.name]); assertEquals("annotation_collision", merged.diagnostics.single().code) }
    @Test fun runtimeConsentAndDedupAreDeterministic() { val runtime = ChillRuntime(ChillConfiguration("https://ingest", "sdk", "v1", Consent.GRANTED), random = Random(0)); assertTrue(runtime.observeAction("cat.adopt", "touch-1", "main")); assertFalse(runtime.observeAction("cat.adopt", "touch-1", "main")); assertEquals("action.duplicate_observation", runtime.diagnostics.last()) }
    @Test fun fileQueueRecoversAndDropsReplayFirst() { val file = File.createTempFile("chill", ".queue").also { it.delete() }; val queue = FileRecordQueue(file, 15); queue.enqueue(QueuedRecord("replay", Kind.REPLAY, 0, "1234567890")); queue.enqueue(QueuedRecord("action", Kind.ACTION, 2, "1234567890")); assertEquals(listOf("action"), FileRecordQueue(file, 15).peek(10).map { it.id }); file.delete() }
    @Test fun exporterAcknowledgesOnlyAfterSuccess() { val queue = MemoryRecordQueue(); val runtime = ChillRuntime(ChillConfiguration("https://ingest", "sdk", "v1", Consent.GRANTED), queue, random = Random(0)); runtime.action("cat.adopt"); var attempts = 0; val exporter = OfflineExporter(runtime, Transport { _, _, _ -> attempts++; if (attempts == 1) throw RetryableExportException(503); ExportResult(emptySet()) }, sleep = {}, jitter = { 0 }); val before = runtime.pending().size; assertEquals(before, exporter.flush()); assertTrue(runtime.pending().isEmpty()); assertEquals(2, attempts) }
    @Test fun propagationRejectsZeroIdsAndThirdParties() { assertEquals(null, TracePropagation.parse("00-00000000000000000000000000000000-00f067aa0ba902b7-01")); assertTrue(TracePropagation.headers("https://api.example.test/cat", setOf("https://api.example.test"), null).containsKey("traceparent")); assertTrue(TracePropagation.headers("https://third.example/cat", setOf("https://api.example.test"), null).isEmpty()) }
}
