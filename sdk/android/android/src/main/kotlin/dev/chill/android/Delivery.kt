package dev.chill.android

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import dev.chill.core.ChillRuntime
import dev.chill.core.OfflineExporter
import dev.chill.core.TracePropagation

public class NetworkAwareDelivery(context: Context, private val exporter: OfflineExporter) : AutoCloseable {
    private val manager = context.getSystemService(ConnectivityManager::class.java)
    private val callback = object : ConnectivityManager.NetworkCallback() { override fun onAvailable(network: Network) { runCatching { exporter.flush() } } }
    init { manager.registerDefaultNetworkCallback(callback) }
    public fun flushIfConnected(): Int { val network = manager.activeNetwork ?: return 0; val capabilities = manager.getNetworkCapabilities(network) ?: return 0; return if (capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)) exporter.flush() else 0 }
    override fun close() { manager.unregisterNetworkCallback(callback) }
}

public fun tracedHeaders(runtime: ChillRuntime, url: String): Map<String, String> = TracePropagation.headers(url, runtime.configuration.trustedTraceOrigins, runtime.currentTrace())
