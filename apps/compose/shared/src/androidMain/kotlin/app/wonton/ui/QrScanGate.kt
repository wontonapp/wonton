package app.wonton.ui

/** Per-camera-session delivery gate; continuous scanners still accept a different code. */
internal class QrScanGate(private val singleScan: Boolean) {
    private var last: String? = null
    var finished: Boolean = false
        private set

    fun accept(code: String): Boolean {
        if (finished || code == last) return false
        last = code
        finished = singleScan
        return true
    }
}
