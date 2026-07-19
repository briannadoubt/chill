# Integrating the Chill Android SDK

Publish or include the `dev.chill:core` Kotlin artifact and `dev.chill:android` AAR. Create one `ChillRuntime` with app-private queue storage, explicit consent, policy version, sampling, and trusted trace origins. All server-side application behavior remains in the Rust Chill service.

```kotlin
val runtime = ChillRuntime(
    ChillConfiguration(
        endpoint = "https://telemetry.internal.example",
        sdkKey = BuildConfig.CHILL_SDK_KEY,
        policyVersion = "internal-v1",
        consent = Consent.GRANTED,
        replaySampleRate = 0.1,
        trustedTraceOrigins = setOf("https://api.internal.example"),
    ),
    FileRecordQueue(File(filesDir, "chill/records.queue")),
)
application.registerActivityLifecycleCallbacks(ChillActivityCallbacks(runtime))
```

Compose applications use `ChillProvider`, `ChillAnnotation`, `ChillPage`, `Modifier.chillImpression`, and `Modifier.chillAction`. View applications use `chillImpression` and `chillAction`. Domain functions use `@ChillActivity`, `@ChillEvent`, and `runtime.activity(name) { … }` so completion, failures, and cancellation remain paired.

`NetworkAwareDelivery` flushes the crash-safe bounded queue only on validated networks. `tracedHeaders` injects W3C `traceparent` only for an exact trusted origin; it never forwards baggage. Retryable responses preserve IDs and only a successful acknowledgement removes records.

`ReplayRecorder` emits source-masked view structure, geometry, gestures, viewport changes, monotonic offsets, and capacity gaps. It never reads `TextView.text`, content descriptions, input values, image pixels, custom drawing, request paths, headers, bodies, or exception messages.
