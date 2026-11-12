package app.wonton.android

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import app.wonton.protocol.DeviceNetwork

/** 把系统默认网络的变化报给核心。壳不做判断（ADR-0032）：换了网络、同一网络换了地址，都报。 */
class NetworkWatcher(context: Context, private val report: (DeviceNetwork) -> Unit) {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private var last: DeviceNetwork? = null

    private val callback = object : ConnectivityManager.NetworkCallback() {
        override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) {
            // 信号强度变化也会走到这里，类型不变就不算变化。
            val now = capabilities.deviceNetwork()
            if (now != last) changed(now)
        }

        override fun onLinkPropertiesChanged(network: Network, properties: LinkProperties) {
            last?.let(::changed)
        }

        override fun onLost(network: Network) {
            changed(DeviceNetwork.NONE)
        }
    }

    fun start() {
        // 没有网络时回调不会来。
        if (connectivity.activeNetwork == null) changed(DeviceNetwork.NONE)
        connectivity.registerDefaultNetworkCallback(callback)
    }

    fun stop() {
        connectivity.unregisterNetworkCallback(callback)
    }

    private fun changed(network: DeviceNetwork) {
        last = network
        report(network)
    }

    private fun NetworkCapabilities.deviceNetwork(): DeviceNetwork = when {
        hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> DeviceNetwork.WIFI
        hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> DeviceNetwork.CELLULAR
        else -> DeviceNetwork.OTHER
    }
}
