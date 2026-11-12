//! iroh 读 Android 的系统 DNS 要 JVM 和 Application（`install_android_jni_context`）。没装时 iroh 退回默认的公共
//! DNS，在国内靠不住。

use jni::JNIEnv;
use jni::objects::JObject;

/// `WontonApplication.installSystemDns()`。UniFFI 经 JNA 加载这个库，不走 `JNI_OnLoad`，所以由 Kotlin 显式调一次。
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_wonton_android_WontonApplication_installSystemDns(
    env: JNIEnv<'_>,
    application: JObject<'_>,
) {
    let installed = env.get_java_vm().and_then(|vm| {
        let application = env.new_global_ref(application)?;
        // SAFETY: 全局引用下面故意不释放，JavaVM 活到进程结束，都满足“指针一直有效”的要求。
        unsafe {
            iroh::dns::install_android_jni_context(
                vm.get_java_vm_pointer().cast(),
                application.as_obj().as_raw().cast(),
            );
        }
        std::mem::forget(application);
        Ok(())
    });
    if let Err(e) = installed {
        tracing::warn!("安装系统 DNS 失败，iroh 用默认 DNS：{e}");
    }
}
