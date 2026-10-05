# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile
# The key plugin is found by NAME from the Rust side, and its commands and arguments by reflection: shrinking must
# not rename or remove any of it. Yubico's library is reached the same way by its own connection classes.
-keep class com.anytwokeys.keylink.** { *; }
-keep class com.yubico.yubikit.** { *; }
-dontwarn com.yubico.yubikit.**
-dontwarn edu.umd.cs.findbugs.annotations.**
-dontwarn org.slf4j.**
