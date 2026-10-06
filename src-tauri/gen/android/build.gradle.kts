buildscript {
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        classpath("com.android.tools.build:gradle:8.11.0")
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:1.9.25")
    }
}

allprojects {
    repositories {
        google()
        mavenCentral()
    }
}

tasks.register("clean").configure {
    delete("build")
}


// 官方插件也是 Android library，统一使用 A0 已确认的工具版本，避免各模块自行补装 SDK。
subprojects {
    pluginManager.withPlugin("com.android.library") {
        extensions.configure<com.android.build.api.dsl.LibraryExtension> {
            buildToolsVersion = "36.0.0"
            ndkVersion = "30.0.16248370"
        }
    }
}
