package dev.chill.core

@Target(AnnotationTarget.FUNCTION)
@Retention(AnnotationRetention.BINARY)
public annotation class ChillActivity(val name: String)

@Target(AnnotationTarget.FUNCTION)
@Retention(AnnotationRetention.BINARY)
public annotation class ChillEvent(val name: String)

public enum class Classification { PUBLIC, INTERNAL, PSEUDONYMOUS_IDENTIFIER }

public data class AnnotationKey<T : Any>(val name: String, val classification: Classification = Classification.INTERNAL) {
    init { require(Regex("^[a-z][a-z0-9_]*(?:\\.[a-z][a-z0-9_]*)*$").matches(name)) { "invalid annotation key: $name" } }
}

public data class ContextDiagnostic(val code: String, val key: String, val kept: Any?, val rejected: Any)

public class AnnotationContext private constructor(
    public val values: Map<String, Any>,
    public val classifications: Map<String, Classification>,
    public val diagnostics: List<ContextDiagnostic>,
) {
    public companion object { public fun empty(): AnnotationContext = AnnotationContext(emptyMap(), emptyMap(), emptyList()) }

    public fun <T : Any> with(key: AnnotationKey<T>, value: T): AnnotationContext {
        if (value !is String && value !is Number && value !is Boolean && !(value is List<*> && value.all { it is String })) {
            return AnnotationContext(values, classifications, diagnostics + ContextDiagnostic("annotation_invalid", key.name, null, value))
        }
        if (key.name in values) return AnnotationContext(values, classifications, diagnostics + ContextDiagnostic("annotation_collision", key.name, values[key.name], value))
        return AnnotationContext(values + (key.name to value), classifications + (key.name to key.classification), diagnostics)
    }

    public fun mergeDescendant(descendant: AnnotationContext): AnnotationContext {
        var result = this
        descendant.values.forEach { (name, value) -> result = result.with(AnnotationKey(name, descendant.classifications[name] ?: Classification.INTERNAL), value) }
        return AnnotationContext(result.values, result.classifications, result.diagnostics + descendant.diagnostics)
    }
}
