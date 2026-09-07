plugins { id("com.android.library"); id("org.jetbrains.kotlin.plugin.compose") }

android {
    namespace = "dev.chill.android"
    compileSdk = 37
    defaultConfig { minSdk = 26; consumerProguardFiles("consumer-rules.pro") }
    buildFeatures { compose = true; buildConfig = false }
    testOptions { unitTests.isIncludeAndroidResources = true }
    lint { abortOnError = true; warningsAsErrors = true }
}

dependencies {
    api(project(":core"))
    implementation(platform("androidx.compose:compose-bom:2026.08.00"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.runtime:runtime")
    implementation("androidx.core:core-ktx:1.19.0")
    testImplementation("junit:junit:4.13.2")
}
