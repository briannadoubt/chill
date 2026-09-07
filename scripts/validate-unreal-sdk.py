#!/usr/bin/env python3
"""Validate the source-only Unreal plugin and its repository integration."""

from __future__ import annotations

import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
PLUGIN = ROOT / "sdk" / "unreal"
RUNTIME = PLUGIN / "Source" / "ChillObservability"
TESTS = PLUGIN / "Source" / "ChillObservabilityTests"


def load_json(path: Path) -> object:
    return json.loads(path.read_text(encoding="utf-8"))


def runtime_sources() -> list[Path]:
    return sorted(
        list((RUNTIME / "Public").glob("*.h")) + list((RUNTIME / "Private").glob("*.cpp"))
    )


def validate() -> list[str]:
    errors: list[str] = []
    required = [
        PLUGIN / "Chill.uplugin",
        PLUGIN / "README.md",
        PLUGIN / "CHANGELOG.md",
        RUNTIME / "ChillObservability.Build.cs",
        RUNTIME / "Public" / "ChillClient.h",
        RUNTIME / "Public" / "ChillSubsystem.h",
        RUNTIME / "Public" / "ChillStore.h",
        RUNTIME / "Private" / "ChillClient.cpp",
        RUNTIME / "Private" / "ChillSubsystem.cpp",
        RUNTIME / "Private" / "ChillStore.cpp",
        TESTS / "ChillObservabilityTests.Build.cs",
        TESTS / "Private" / "ChillClientTests.cpp",
        PLUGIN / "Samples" / "BasicIntegration" / "BasicChillBootstrap.cpp",
        ROOT / "docs" / "integrating-unreal.md",
        ROOT / "docs" / "architecture" / "adr-0031-unreal-sdk.md",
        ROOT / "backend" / "testdata" / "unreal-sdk-otlp.json",
        ROOT / "backend" / "crates" / "chill-normalize" / "tests" / "unreal_sdk.rs",
    ]
    for path in required:
        if not path.is_file():
            errors.append(f"missing Unreal plugin file: {path.relative_to(ROOT)}")
    if errors:
        return errors

    descriptor = load_json(PLUGIN / "Chill.uplugin")
    assert isinstance(descriptor, dict)
    version = (ROOT / "VERSION").read_text(encoding="utf-8").strip()
    toolchains = load_json(ROOT / "ci" / "toolchains.json")
    assert isinstance(toolchains, dict)
    unreal_toolchain = toolchains.get("unreal", {})

    coordinated_package_version = version.split("-", 1)[0]
    if descriptor.get("VersionName") != coordinated_package_version:
        errors.append("Unreal plugin VersionName and VERSION disagree")
    # A .uplugin EngineVersion pins the plugin to one exact engine build and makes
    # newer engines refuse to load it, so a source plugin must not declare one. The
    # supported minimum lives in ci/toolchains.json and the integration guide.
    if "EngineVersion" in descriptor:
        errors.append("Unreal plugin pins EngineVersion; newer engines will refuse to load it")
    minimum_version = unreal_toolchain.get("minimum_version")
    if not minimum_version:
        errors.append("Unreal toolchain manifest does not record a minimum engine version")
    else:
        guide = (ROOT / "docs" / "integrating-unreal.md").read_text(encoding="utf-8")
        if minimum_version not in guide:
            errors.append("integration guide does not state the supported engine minimum")
    if descriptor.get("CanContainContent") is not False:
        errors.append("Unreal plugin declares content; it must ship as source only")

    modules = {module.get("Name"): module for module in descriptor.get("Modules", [])}
    if set(modules) != {"ChillObservability", "ChillObservabilityTests"}:
        errors.append("Unreal plugin module set changed")
    else:
        if modules["ChillObservability"].get("Type") != "Runtime":
            errors.append("Unreal runtime module is not a Runtime module")
        if modules["ChillObservabilityTests"].get("Type") != "UncookedOnly":
            errors.append("Unreal test module ships in cooked builds")

    build_rules = (RUNTIME / "ChillObservability.Build.cs").read_text(encoding="utf-8")
    # The dependency boundary is the plugin's supply-chain surface: first-party
    # engine modules only, so there is nothing third-party to audit or vendor.
    for forbidden in ("AddThirdPartyPrivateStaticDependencies", "PublicAdditionalLibraries"):
        if forbidden in build_rules:
            errors.append(f"Unreal runtime module links native libraries: {forbidden}")
    for required_module in ('"Core"', '"CoreUObject"', '"Engine"', '"HTTP"'):
        if required_module not in build_rules:
            errors.append(f"Unreal runtime module dependency missing: {required_module}")

    source = "\n".join(path.read_text(encoding="utf-8") for path in runtime_sources())
    forbidden_tokens = {
        "dlopen": "native FFI",
        "FPlatformProcess::GetDllHandle": "native FFI",
        "GetName()": "automatic object-name capture",
        "GetPathName()": "automatic asset-path capture",
        "GetActorLocation": "automatic transform capture",
        "bool Track(": "generic tracking API",
    }
    for token, label in forbidden_tokens.items():
        if token in source:
            errors.append(f"Unreal runtime contains forbidden {label}: {token}")

    binaries = [
        path
        for path in PLUGIN.rglob("*")
        if path.suffix.lower() in {".a", ".dll", ".dylib", ".so", ".lib", ".uasset", ".umap"}
    ]
    if binaries:
        errors.append("Unreal plugin contains native or content binaries")

    # The durable queue must never be able to reach a credential.
    store_source = (RUNTIME / "Private" / "ChillStore.cpp").read_text(encoding="utf-8")
    store_header = (RUNTIME / "Public" / "ChillStore.h").read_text(encoding="utf-8")
    for token in ("SdkKey", "Authorization", "Bearer"):
        if token in store_source or token in store_header:
            errors.append(f"Unreal durable store can access credential token {token}")

    transport = (RUNTIME / "Private" / "ChillSubsystem.cpp").read_text(encoding="utf-8")
    for required_token in (
        'SetHeader(TEXT("Authorization"), TEXT("Bearer ") + Client->GetSdkKey())',
        "ResponseCode >= 200",
        "Client->Acknowledge(Acknowledged)",
        "CancelRequest()",
    ):
        if required_token not in transport:
            errors.append(f"Unreal transport invariant missing: {required_token}")

    client_source = (RUNTIME / "Private" / "ChillClient.cpp").read_text(encoding="utf-8")
    for required_token in (
        "Store->Purge()",
        'EmitLocked(TEXT("session"), TEXT("start")',
        'EmitLocked(TEXT("session"), TEXT("end")',
        "MaximumSafeSequence",
        "AnnotationDefinitions",
        'ChillOtlp::Text(TEXT("chill.clock.boot_id"), BootId)',
    ):
        if required_token not in client_source:
            errors.append(f"Unreal client invariant missing: {required_token}")

    record_source = (RUNTIME / "Private" / "ChillRecord.cpp").read_text(encoding="utf-8")
    for required_token in ('StringValue(TEXT("unreal"))', "dev.chill.unreal", 'StringValue(TEXT("cpp"))'):
        if required_token not in record_source:
            errors.append(f"Unreal envelope identity missing: {required_token}")

    source_bytes = sum(path.stat().st_size for path in runtime_sources())
    if source_bytes > 256 * 1024:
        errors.append(f"Unreal runtime source budget exceeded: {source_bytes} > {256 * 1024}")

    test_source = (TESTS / "Private" / "ChillClientTests.cpp").read_text(encoding="utf-8")
    if test_source.count("IMPLEMENT_SIMPLE_AUTOMATION_TEST") < 11:
        errors.append("Unreal plugin has fewer than eleven contract tests")

    normalizer_test = (
        ROOT / "backend" / "crates" / "chill-normalize" / "tests" / "unreal_sdk.rs"
    ).read_text(encoding="utf-8")
    for required_token in (
        "unreal-sdk-otlp.json",
        "unreal_generated_otlp_passes_the_production_normalizer",
        "validator.validate(&record.canonical)",
    ):
        if required_token not in normalizer_test:
            errors.append(f"Unreal normalizer contract missing: {required_token}")

    components = load_json(ROOT / "ci" / "components.json")
    assert isinstance(components, dict)
    unreal_components = [
        component
        for component in components.get("components", [])
        if component.get("id") == "unreal"
    ]
    if len(unreal_components) != 1 or unreal_components[0].get("state") != "active":
        errors.append("Unreal is not registered as one active component")

    release_script = (ROOT / "scripts" / "prepare-release-assets.sh").read_text(encoding="utf-8")
    if "chill-unreal-$version.tar.gz" not in release_script:
        errors.append("release assets omit the Unreal source package")

    workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    if "scripts/validate-unreal-sdk.py" not in workflow:
        errors.append("continuous integration omits Unreal plugin validation")
    return errors


def main() -> int:
    errors = validate()
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    sources = runtime_sources()
    print(
        "Unreal SDK validation passed: "
        f"{len(sources)} runtime files, "
        f"{sum(path.stat().st_size for path in sources)} source bytes"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
