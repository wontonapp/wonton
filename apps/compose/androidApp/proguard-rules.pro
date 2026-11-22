# R8 默认看不到的反射用法。签名和字段名是运行时按字符串找的，改名或删掉就崩。

# UniFFI 生成的绑定经 JNA 直连：库先 Native.load，接口里的方法名当 C 符号，RustBuffer 之类的结构体按字段名算偏移，
# 回调接口由 Rust 反向调用。按“实现/继承了什么”保留，生成的类型本身照常缩、照常改名。
-dontwarn java.awt.**
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.Library { *; }
-keep class * extends com.sun.jna.Structure { *; }
-keep class * implements com.sun.jna.Callback { *; }

# WorkManager 把排队的工作（推送动作的 PushActionWorker）按类名写进数据库，之后按类名反射 new；
# 入队时只给了类型，R8 会把没人直接 new 的构造删掉。按继承保留构造，类名跟着构造一起留下。
-keep class * extends androidx.work.ListenableWorker { <init>(...); }

# Room 按类名（<库类>_Impl）找生成的实现再反射 new 无参构造，WorkManager 的数据库在进程一起来就初始化，
# 被改名或删构造就直接起不来。生成的实现都继承 RoomDatabase，按继承保留，构造留着。
-keep class * extends androidx.room.RoomDatabase { <init>(); }

# Rust 的 rustls-platform-verifier 用 JNI 按类名、方法名、字段名调这几种类（见
# rustls-platform-verifier 的 src/verification/android.rs）。
-keep class org.rustls.platformverifier.** { *; }
