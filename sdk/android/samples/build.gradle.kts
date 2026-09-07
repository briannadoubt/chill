plugins { id("com.android.library"); id("org.jetbrains.kotlin.plugin.compose") }
android {
    namespace = "dev.chill.samples"
    compileSdk = 37
    defaultConfig { minSdk = 26 }
    buildFeatures { compose = true; buildConfig = false }
    lint { abortOnError = true; warningsAsErrors = true; disable += "GradleDependency" }
}
dependencies {
    implementation(project(":android"))
    implementation(platform("androidx.compose:compose-bom:2026.08.00"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
}
