package app.wonton.ui

import app.wonton.ui.design.WontonTheme

/** All inputs are physical pixels (velocity in pixels/second); tokens are dp and dp/second. */
internal fun commitsGesture(distance: Float, velocity: Float, threshold: Float, density: Float): Boolean =
    distance >= threshold * density ||
        (distance >= WontonTheme.gesture.flickDistance * density &&
            velocity >= WontonTheme.gesture.flickVelocity * density)
