package dev.chill.samples.views

import android.app.Activity
import android.os.Bundle
import android.widget.Button
import dev.chill.android.chillAction
import dev.chill.core.ChillRuntime
import dev.chill.samples.R

public abstract class InboxActivity : Activity() {
    public abstract val chill: ChillRuntime
    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        setContentView(Button(this).apply { setText(R.string.compose); chillAction(chill, "message.compose", "compose") })
    }
}
