# JNA and the UniFFI bindings are reached through reflection and from native code: keep them
# whole in apps that shrink with R8.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class uniffi.** { *; }
-dontwarn java.awt.**
