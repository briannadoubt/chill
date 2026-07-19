package dev.chill.android

import android.app.Activity
import android.os.Handler
import android.os.HandlerThread
import android.view.FrameMetrics
import android.view.Window
import dev.chill.core.ChillRuntime

public class AndroidDiagnostics(private val runtime: ChillRuntime) : AutoCloseable {
    private val previous = Thread.getDefaultUncaughtExceptionHandler()
    private val thread = HandlerThread("chill-frame-metrics").also { it.start() }
    private val windows = mutableSetOf<Window>()
    private val listener = Window.OnFrameMetricsAvailableListener { _, metrics, dropped ->
        runtime.event("app.frame", mapOf("duration_nano" to metrics.getMetric(FrameMetrics.TOTAL_DURATION), "dropped_since_last" to dropped))
    }

    public fun install() {
        Thread.setDefaultUncaughtExceptionHandler { worker, error ->
            runtime.event("app.crash", mapOf("error_type" to error.javaClass.simpleName, "thread" to worker.name.take(64)))
            runCatching { previous?.uncaughtException(worker, error) }
        }
    }

    public fun observe(activity: Activity) { activity.window.addOnFrameMetricsAvailableListener(listener, Handler(thread.looper)); windows += activity.window }
    override fun close() { windows.forEach { it.removeOnFrameMetricsAvailableListener(listener) }; windows.clear(); if (Thread.getDefaultUncaughtExceptionHandler() !== previous) Thread.setDefaultUncaughtExceptionHandler(previous); thread.quitSafely() }
}
