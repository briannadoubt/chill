package dev.chill.core

internal object Json {
    fun encode(value: Any?): String = when (value) {
        null -> "null"
        is String -> "\"${value.replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", "\\n").replace("\r", "\\r")}\""
        is Boolean, is Number -> value.toString()
        is Enum<*> -> encode(value.name.lowercase())
        is Map<*, *> -> value.entries.joinToString(",", "{", "}") { encode(it.key.toString()) + ":" + encode(it.value) }
        is Iterable<*> -> value.joinToString(",", "[", "]") { encode(it) }
        else -> encode(value.toString())
    }

    fun record(record: BehaviorRecord): String = encode(
        linkedMapOf(
            "schema_version" to "1.0.0", "schema_url" to "https://schemas.chill.dev/behavior/v1/envelope.schema.json",
            "record_id" to record.recordId, "subject_id" to record.subjectId, "kind" to record.kind, "operation" to record.operation,
            "name" to record.name, "clock" to mapOf("wall_unix_nano" to record.clock.wallUnixNano, "monotonic_nano" to record.clock.monotonicNano, "sequence_number" to record.clock.sequenceNumber),
            "source" to mapOf("platform" to "android", "sdk_version" to "0.1.0"),
            "context" to buildMap { put("session_id", record.sessionId); put("replay_id", record.replayId); record.page?.let { put("page", mapOf("instance_id" to it.instanceId, "path" to it.path, "path_instance_ids" to it.pathInstanceIds)) } },
            "annotations" to record.annotations, "privacy" to mapOf("consent" to record.consent, "policy_version" to record.policyVersion, "redaction_state" to if (record.kind == Kind.REPLAY) "masked" else "none"),
            "trace" to record.trace?.let { mapOf("trace_id" to it.traceId, "span_id" to it.spanId, "sampled" to it.sampled) }, "payload" to record.payload,
        )
    )
}
