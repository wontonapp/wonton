//! App 启动时要从 JVM 拿的东西。UniFFI 经 JNA 加载这个库，不走 `JNI_OnLoad`，所以由 `WontonApplication.onCreate`
//! 显式调这里的本地方法，赶在核心发出任何网络请求之前。

use jni::objects::JObject;
use jni::{EnvUnowned, Outcome};

/// `WontonApplication.installSystemDns()`：iroh 读 Android 的系统 DNS 要 JVM 和 Application
/// （`install_android_jni_context`）。没装时 iroh 退回默认的公共 DNS，在国内靠不住。
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_wonton_android_WontonApplication_installSystemDns<'caller>(
    mut env: EnvUnowned<'caller>,
    application: JObject<'caller>,
) {
    let installed = env.with_env(|env| -> jni::errors::Result<()> {
        let vm = env.get_java_vm()?;
        let application = env.new_global_ref(&application)?;
        // SAFETY: 全局引用下面故意不释放，JavaVM 活到进程结束，都满足“指针一直有效”的要求。
        unsafe {
            iroh::dns::install_android_jni_context(
                vm.get_raw().cast(),
                application.as_raw().cast(),
            );
        }
        std::mem::forget(application);
        Ok(())
    });
    if let Err(reason) = outcome(installed.into_outcome()) {
        tracing::warn!(
            "could not install the system DNS; iroh falls back to the default DNS: {reason}"
        );
    }
}

/// `WontonApplication.installCertificateVerifier()`：reqwest 和 matrix-sdk 用 rustls-platform-verifier 按系统证书库
/// 校验 HTTPS 证书（用户装的 CA 也认），它在 Android 上要经 JNI 调 App 里的 `org.rustls.platformverifier` 组件。
/// 结果记进 [`wonton_matrix::platform_tls`]：失败或没调时，连 HTTPS 的地方报传输不可用，而不是校验时 panic。
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_wonton_android_WontonApplication_installCertificateVerifier<
    'caller,
>(
    mut env: EnvUnowned<'caller>,
    application: JObject<'caller>,
) {
    let installed = env.with_env(|env| -> jni::errors::Result<Result<(), String>> {
        let result = rustls_platform_verifier::android::init_with_env(env, application)
            .map_err(|error| error.to_string());
        // 失败时 JVM 上可能挂着异常；不清掉的话回到 Kotlin 就抛出来，App 起不来。
        if result.is_err() {
            env.exception_clear();
        }
        Ok(result)
    });
    let result = outcome(installed.into_outcome()).and_then(|result| result);
    if let Err(reason) = &result {
        tracing::error!("could not initialize the system certificate verifier: {reason}");
    }
    wonton_matrix::platform_tls::record_init(result);
}

fn outcome<T>(outcome: Outcome<T, jni::errors::Error>) -> Result<T, String> {
    match outcome {
        Outcome::Ok(value) => Ok(value),
        Outcome::Err(error) => Err(error.to_string()),
        Outcome::Panic(_) => Err("panicked".to_owned()),
    }
}
