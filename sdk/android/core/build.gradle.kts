plugins { id("org.jetbrains.kotlin.jvm") }

kotlin { jvmToolchain(17); compilerOptions { allWarningsAsErrors.set(true); progressiveMode.set(true) } }

dependencies { testImplementation(kotlin("test")); testImplementation("org.junit.jupiter:junit-jupiter:5.13.4") }
tasks.test { useJUnitPlatform() }
