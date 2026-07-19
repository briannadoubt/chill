package dev.chill.samples.compose

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import dev.chill.android.ChillPage
import dev.chill.android.chillAction

@Composable
public fun MainScreen() {
    ChillPage("inbox") {
        Column {
            Button(onClick = {}, modifier = Modifier.chillAction("message.compose", "compose")) { Text("Compose") }
        }
    }
}
