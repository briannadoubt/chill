package dev.chill.android

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.changedToUp
import androidx.compose.ui.layout.onGloballyPositioned
import dev.chill.core.AnnotationContext
import dev.chill.core.AnnotationKey
import dev.chill.core.ChillRuntime

public val LocalChillRuntime = staticCompositionLocalOf<ChillRuntime> { error("ChillProvider is missing") }
public val LocalChillContext = staticCompositionLocalOf { AnnotationContext.empty() }

@Composable public fun ChillProvider(runtime: ChillRuntime, content: @Composable () -> Unit) { CompositionLocalProvider(LocalChillRuntime provides runtime, LocalChillContext provides AnnotationContext.empty(), content = content) }
@Composable public fun <T : Any> ChillAnnotation(key: AnnotationKey<T>, value: T, content: @Composable () -> Unit) { val context = LocalChillContext.current.with(key, value); CompositionLocalProvider(LocalChillContext provides context, content = content) }
@Composable public fun ChillPage(name: String, content: @Composable () -> Unit) { val runtime = LocalChillRuntime.current; DisposableEffect(runtime, name) { runtime.startPage(name); onDispose { } }; content() }

public fun Modifier.chillImpression(name: String): Modifier = composed { val runtime = LocalChillRuntime.current; val context = LocalChillContext.current; var emitted = false; onGloballyPositioned { if (!emitted && it.isAttached) { emitted = true; runtime.setContext(context); runtime.impression(name, mapOf("width" to it.size.width, "height" to it.size.height)) } } }
public fun Modifier.chillAction(name: String, elementId: String): Modifier = composed { val runtime = LocalChillRuntime.current; val context = LocalChillContext.current; pointerInput(runtime, name, elementId) { awaitPointerEventScope { while (true) { val event = awaitPointerEvent(); val change = event.changes.firstOrNull(); if (change?.changedToUp() == true) { runtime.setContext(context); runtime.observeAction(name, "${change.uptimeMillis}:${change.id.value}", runtime.currentPage()?.instanceId ?: "compose", mapOf("element_id" to elementId, "input" to "touch")) } } } } }
