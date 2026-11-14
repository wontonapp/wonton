package app.wonton.client

import com.sun.jna.CallbackThreadInitializer
import com.sun.jna.Native

/**
 * Keeps Rust future callbacks attached until their native worker thread exits.
 *
 * UniFFI resumes Kotlin continuations synchronously from JNA callbacks. Detaching
 * in JNA's callback return path can therefore run while the continuation still
 * has Java frames on the stack; ART aborts that detach. JNA's thread-local
 * cleanup detaches safely after the native thread has unwound.
 */
fun configureUniFfiCallbackThreads() {
    Native.setCallbackThreadInitializer(
        uniffiRustFutureContinuationCallbackImpl,
        CallbackThreadInitializer(true, false),
    )
}
