plugins {
    kotlin("jvm") version "2.0.21"
    application
}

group = "ai.meshllm.example"
version = "0.1.0"

repositories {
    mavenCentral()
}

dependencies {
    implementation(kotlin("stdlib"))
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.7.3")
}

// Include parent binding sources directly — avoids triggering the Android NDK native build
sourceSets {
    main {
        kotlin {
            srcDir("../../src/main/kotlin")
        }
    }
}

application {
    mainClass.set("ai.meshllm.example.ExampleMainKt")
}
