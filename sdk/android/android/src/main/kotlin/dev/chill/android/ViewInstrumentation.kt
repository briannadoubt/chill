package dev.chill.android

import android.app.Activity
import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import android.widget.CompoundButton
import android.widget.TextView
import android.text.Editable
import android.text.TextWatcher
import dev.chill.core.AnnotationContext
import dev.chill.core.ChillRuntime

public fun View.chillAction(runtime: ChillRuntime, name: String, elementId: String, context: AnnotationContext = AnnotationContext.empty(), onActivate: (View) -> Unit = {}) {
    setOnClickListener { view -> runtime.setContext(context); runtime.observeAction(name, "${view.id}:${android.os.SystemClock.uptimeMillis()}", runtime.currentPage()?.instanceId ?: "view", mapOf("element_id" to elementId, "role" to view.javaClass.simpleName)); onActivate(view) }
}

public fun View.chillImpression(runtime: ChillRuntime, name: String, elementId: String, context: AnnotationContext = AnnotationContext.empty()) {
    addOnAttachStateChangeListener(object : View.OnAttachStateChangeListener { override fun onViewAttachedToWindow(view: View) { runtime.setContext(context); runtime.impression(name, mapOf("element_id" to elementId, "width" to view.width, "height" to view.height)) }; override fun onViewDetachedFromWindow(view: View) = Unit })
}

public fun TextView.chillForm(runtime: ChillRuntime, name: String, elementId: String, context: AnnotationContext = AnnotationContext.empty()): TextWatcher {
    val watcher = object : TextWatcher { override fun beforeTextChanged(value: CharSequence?, start: Int, count: Int, after: Int) = Unit; override fun onTextChanged(value: CharSequence?, start: Int, before: Int, count: Int) { runtime.setContext(context); runtime.event(name, mapOf("element_id" to elementId, "value_captured" to false, "length_bucket" to when ((value?.length ?: 0)) { in 0..4 -> "0-4"; in 5..16 -> "5-16"; else -> "17+" })) }; override fun afterTextChanged(value: Editable?) = Unit }
    addTextChangedListener(watcher); return watcher
}

public open class ChillActivityCallbacks(private val runtime: ChillRuntime) : android.app.Application.ActivityLifecycleCallbacks {
    override fun onActivityCreated(activity: Activity, state: Bundle?) { runtime.startPage(activity.javaClass.simpleName.lowercase().replace(Regex("[^a-z0-9_-]"), "-")) }
    override fun onActivityStarted(activity: Activity) = Unit; override fun onActivityResumed(activity: Activity) { runtime.event("app.foreground") }; override fun onActivityPaused(activity: Activity) { runtime.event("app.background") }; override fun onActivityStopped(activity: Activity) = Unit; override fun onActivitySaveInstanceState(activity: Activity, state: Bundle) = Unit; override fun onActivityDestroyed(activity: Activity) = Unit
}

internal fun role(view: View): String = when (view) { is CompoundButton -> "toggle"; is TextView -> "text"; is ViewGroup -> "container"; else -> view.javaClass.simpleName }
