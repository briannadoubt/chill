package dev.chill.android

import android.graphics.Rect
import android.view.View
import android.view.ViewGroup
import dev.chill.core.ChillRuntime

public data class ReplayNode(val role: String, val resourceId: String?, val frame: Rect, val masked: Boolean = true, val children: List<ReplayNode> = emptyList())
public object ViewReplayCapture {
    public fun snapshot(root: View): ReplayNode { val location = IntArray(2); root.getLocationOnScreen(location); val children = if (root is ViewGroup) (0 until root.childCount).map { snapshot(root.getChildAt(it)) } else emptyList(); val id = if (root.id == View.NO_ID) null else runCatching { root.resources.getResourceEntryName(root.id) }.getOrNull(); return ReplayNode(role(root), id, Rect(location[0], location[1], location[0] + root.width, location[1] + root.height), children = children) }
}

public class ReplayRecorder(private val runtime: ChillRuntime, private val maximumChunks: Int = 1_000) {
    private var sequence = 0; private var dropped = 0; private val started = System.nanoTime()
    public fun snapshot(root: View) { capture("snapshot", mapOf("tree" to ViewReplayCapture.snapshot(root).toString())) }
    public fun gesture(x: Float, y: Float, type: String) { capture("gesture", mapOf("x" to x.toInt(), "y" to y.toInt(), "gesture" to type)) }
    public fun viewport(width: Int, height: Int) { capture("viewport", mapOf("width" to width, "height" to height)) }
    public fun finish() { if (dropped > 0) runtime.replay(mapOf("replay_id" to runtime.replayId, "sequence" to sequence, "type" to "gap", "dropped_chunks" to dropped, "reason" to "capacity", "masking" to "source")) }
    private fun capture(type: String, payload: Map<String, Any>) { if (!runtime.shouldReplay()) return; if (sequence >= maximumChunks) { dropped++; return }; runtime.replay(payload + mapOf("replay_id" to runtime.replayId, "sequence" to sequence++, "monotonic_offset_nano" to (System.nanoTime() - started).toString(), "type" to type, "masking" to "source")) }
}
