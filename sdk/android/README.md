# Chill Android SDK

`core` is a Kotlin/JVM contract runtime and `android` adds Jetpack Compose, Android Views, lifecycle, connectivity, tracing, and source-redacted structural replay adapters. Application backend behavior stays in the Rust Chill service.

The core uses Kotlin annotations and `runtime.activity { }` / `runtime.event(...)` wrappers for domain instrumentation. Compose consumers declare `ChillProvider`, `ChillAnnotation`, `ChillPage`, `Modifier.chillImpression`, and `Modifier.chillAction`. Views consumers use `ChillActivityCallbacks` and view extensions. Automatic capture never reads `TextView.text`, content descriptions, input values, pixels, HTTP paths, headers, or bodies.

Run `./gradlew test lint assembleRelease && scripts/check-budgets.sh` to exercise conformance, Android lint, release packaging, and the 512 KiB direct-artifact budget.
