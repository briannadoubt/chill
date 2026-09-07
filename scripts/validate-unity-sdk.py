#!/usr/bin/env python3
"""Validate the source-only Unity package and its repository integration."""

from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
PACKAGE = ROOT / "sdk" / "unity"


def load_json(path: Path) -> object:
    return json.loads(path.read_text(encoding="utf-8"))


def validate() -> list[str]:
    errors: list[str] = []
    required = [
        PACKAGE / "package.json",
        PACKAGE / "README.md",
        PACKAGE / "CHANGELOG.md",
        PACKAGE / "Runtime" / "Chill.Unity.asmdef",
        PACKAGE / "Runtime" / "ChillClient.cs",
        PACKAGE / "Runtime" / "ChillRuntime.cs",
        PACKAGE / "Runtime" / "ChillStore.cs",
        PACKAGE / "Tests" / "Editor" / "Chill.Unity.Editor.Tests.asmdef",
        PACKAGE / "Tests" / "Editor" / "ChillClientTests.cs",
        PACKAGE / "Samples~" / "BasicIntegration" / "BasicChillBootstrap.cs",
        ROOT / "docs" / "integrating-unity.md",
        ROOT / "docs" / "architecture" / "adr-0030-unity-sdk.md",
        ROOT / "backend" / "testdata" / "unity-sdk-otlp.json",
        ROOT / "backend" / "crates" / "chill-normalize" / "tests" / "unity_sdk.rs",
    ]
    for path in required:
        if not path.is_file():
            errors.append(f"missing Unity package file: {path.relative_to(ROOT)}")
    if errors:
        return errors

    manifest = load_json(PACKAGE / "package.json")
    assert isinstance(manifest, dict)
    version = (ROOT / "VERSION").read_text(encoding="utf-8").strip()
    toolchains = load_json(ROOT / "ci" / "toolchains.json")
    assert isinstance(toolchains, dict)
    unity_toolchain = toolchains.get("unity", {})
    if manifest.get("name") != "com.chill.observability":
        errors.append("Unity package name is not com.chill.observability")
    coordinated_package_version = version.split("-", 1)[0]
    if manifest.get("version") != coordinated_package_version:
        errors.append("Unity package version and VERSION disagree")
    if manifest.get("unity") != unity_toolchain.get("minimum_version"):
        errors.append("Unity package minimum and toolchain manifest disagree")
    if manifest.get("license") != "Apache-2.0":
        errors.append("Unity package does not declare Apache-2.0")
    if manifest.get("dependencies") != {"com.unity.modules.unitywebrequest": "1.0.0"}:
        errors.append("Unity package dependency boundary changed")

    runtime_asmdef = load_json(PACKAGE / "Runtime" / "Chill.Unity.asmdef")
    tests_asmdef = load_json(
        PACKAGE / "Tests" / "Editor" / "Chill.Unity.Editor.Tests.asmdef"
    )
    assert isinstance(runtime_asmdef, dict)
    assert isinstance(tests_asmdef, dict)
    if runtime_asmdef.get("allowUnsafeCode") is not False:
        errors.append("Unity runtime assembly enables unsafe code")
    if runtime_asmdef.get("overrideReferences") is not False:
        errors.append("Unity runtime assembly overrides managed references")
    if tests_asmdef.get("optionalUnityReferences") != ["TestAssemblies"]:
        errors.append("Unity tests are not registered as TestAssemblies")
    if "UNITY_INCLUDE_TESTS" not in tests_asmdef.get("defineConstraints", []):
        errors.append("Unity package tests lack the UNITY_INCLUDE_TESTS guard")

    runtime_files = sorted((PACKAGE / "Runtime").glob("*.cs"))
    runtime_source = "\n".join(
        path.read_text(encoding="utf-8") for path in runtime_files
    )
    forbidden = {
        "DllImport": "native FFI",
        "System.Reflection": "reflection",
        "UnityEditor": "editor-only runtime dependency",
        "gameObject.name": "automatic object-name capture",
        "scene.path": "automatic scene-path capture",
        "public void Track(": "generic tracking API",
    }
    for token, label in forbidden.items():
        if token in runtime_source:
            errors.append(f"Unity runtime contains forbidden {label}: {token}")
    native_artifacts = [
        path
        for path in PACKAGE.rglob("*")
        if path.suffix.lower() in {".a", ".dll", ".dylib", ".so"}
    ]
    if native_artifacts:
        errors.append("Unity package contains native or precompiled binaries")

    store_source = (PACKAGE / "Runtime" / "ChillStore.cs").read_text(
        encoding="utf-8"
    )
    for token in ("SdkKey", "Authorization", "Bearer"):
        if token in store_source:
            errors.append(f"Unity durable store can access credential token {token}")
    runtime_host = (PACKAGE / "Runtime" / "ChillRuntime.cs").read_text(
        encoding="utf-8"
    )
    for required_token in (
        'SetRequestHeader("Authorization", "Bearer " + client.SdkKey)',
        "currentRequest.responseCode >= 200",
        "client.Acknowledge(batch)",
        "currentRequest.Abort()",
    ):
        if required_token not in runtime_host:
            errors.append(f"Unity transport invariant missing: {required_token}")
    client_source = (PACKAGE / "Runtime" / "ChillClient.cs").read_text(
        encoding="utf-8"
    )
    for required_token in (
        "store.Purge()",
        'Emit("session", "start"',
        'Emit("session", "end"',
        "MaximumSafeSequence",
        "annotationDefinitions",
        'ChillOtlp.Text("chill.clock.boot_id", bootId)',
    ):
        if required_token not in client_source:
            errors.append(f"Unity client invariant missing: {required_token}")

    source_bytes = sum(path.stat().st_size for path in runtime_files)
    if source_bytes > 256 * 1024:
        errors.append(
            f"Unity runtime source budget exceeded: {source_bytes} > {256 * 1024}"
        )
    test_source = (PACKAGE / "Tests" / "Editor" / "ChillClientTests.cs").read_text(
        encoding="utf-8"
    )
    if test_source.count("[Test]") < 11:
        errors.append("Unity package has fewer than eleven contract tests")
    normalizer_test = (
        ROOT / "backend" / "crates" / "chill-normalize" / "tests" / "unity_sdk.rs"
    ).read_text(encoding="utf-8")
    for required_token in (
        "unity-sdk-otlp.json",
        "unity_generated_otlp_passes_the_production_normalizer",
        "validator.validate(&record.canonical)",
    ):
        if required_token not in normalizer_test:
            errors.append(f"Unity normalizer contract missing: {required_token}")

    components = load_json(ROOT / "ci" / "components.json")
    assert isinstance(components, dict)
    unity_components = [
        component
        for component in components.get("components", [])
        if component.get("id") == "unity"
    ]
    if len(unity_components) != 1 or unity_components[0].get("state") != "active":
        errors.append("Unity is not registered as one active component")
    release_script = (ROOT / "scripts" / "prepare-release-assets.sh").read_text(
        encoding="utf-8"
    )
    if "chill-unity-$version.tar.gz" not in release_script:
        errors.append("release assets omit the Unity source package")
    ignore_rules = (ROOT / ".gitignore").read_text(encoding="utf-8")
    if "!sdk/unity/Samples~/" not in ignore_rules:
        errors.append("Unity UPM samples remain excluded by the backup-file ignore rule")
    workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
        encoding="utf-8"
    )
    if "scripts/validate-unity-sdk.py" not in workflow:
        errors.append("continuous integration omits Unity package validation")
    return errors


def main() -> int:
    errors = validate()
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    runtime_bytes = sum(
        path.stat().st_size for path in (PACKAGE / "Runtime").glob("*.cs")
    )
    print(
        "Unity SDK validation passed: "
        f"{len(list((PACKAGE / 'Runtime').glob('*.cs')))} runtime files, "
        f"{runtime_bytes} source bytes"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
