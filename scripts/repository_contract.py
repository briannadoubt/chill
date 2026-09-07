#!/usr/bin/env python3
"""Generate and verify Chill's repository and contract integrity manifests."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
GENERATED_DIRECTORY = ROOT / "contracts" / "generated"
GENERATED_MANIFEST = GENERATED_DIRECTORY / "manifest.v1.json"
CONTRACT_PREFIXES = (
    "budgets",
    "conformance",
    "contracts",
    "examples/behavior",
    "examples/otlp",
    "policies",
    "schemas",
)
CONTRACT_SUFFIXES = {".json", ".py"}
SEMVER = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)
STABLE_SEMVER = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
REMOTE_ACTION = re.compile(
    r"^\s*uses:\s*([^@\s]+)@([0-9a-f]{40})\s+#\s+(v[0-9][0-9A-Za-z.-]*)\s*$"
)
COMMUNITY_FILES = (
    "CODE_OF_CONDUCT.md",
    "CONTRIBUTING.md",
    "GOVERNANCE.md",
    "LICENSE",
    "NOTICE",
    "SECURITY.md",
    "SUPPORT.md",
    ".github/ISSUE_TEMPLATE/bug_report.yml",
    ".github/ISSUE_TEMPLATE/feature_request.yml",
    ".github/PULL_REQUEST_TEMPLATE.md",
)


class RepositoryContractError(ValueError):
    pass


def load_json(path: Path) -> Any:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def canonical_json(value: Any) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def generated_manifest_path(contract_major: int) -> Path:
    return GENERATED_DIRECTORY / f"manifest.v{contract_major}.json"


def _belongs_to_contract_major(relative: Path, contract_major: int) -> bool:
    value = relative.as_posix()
    versioned_prefixes = (
        "budgets/sdk/",
        "conformance/",
        "contracts/budgets/",
        "contracts/conformance/",
    )
    for prefix in versioned_prefixes:
        if value.startswith(prefix):
            remainder = value.removeprefix(prefix)
            version = remainder.split("/", 1)[0]
            if re.fullmatch(r"v[0-9]+", version):
                return version == f"v{contract_major}"
    return contract_major == 1


def tracked_contract_paths(contract_major: int = 1) -> list[Path]:
    command = ["git", "ls-files", "-z", "--", *CONTRACT_PREFIXES]
    result = subprocess.run(
        command,
        cwd=ROOT,
        check=True,
        capture_output=True,
    )
    paths: list[Path] = []
    for raw in result.stdout.split(b"\0"):
        if not raw:
            continue
        relative = Path(raw.decode())
        if relative.parent == GENERATED_DIRECTORY.relative_to(ROOT):
            continue
        if relative.suffix not in CONTRACT_SUFFIXES:
            continue
        if not _belongs_to_contract_major(relative, contract_major):
            continue
        path = ROOT / relative
        if path.is_symlink() or not path.is_file():
            raise RepositoryContractError(
                f"contract input must be a regular tracked file: {relative}"
            )
        paths.append(relative)
    if not paths:
        raise RepositoryContractError("no tracked contract inputs found")
    return sorted(paths, key=lambda path: path.as_posix())


def manifest_payload(contract_major: int = 1) -> dict[str, Any]:
    suite = load_json(
        ROOT / "conformance" / f"v{contract_major}" / "manifest.json"
    )
    entries: list[dict[str, Any]] = []
    for relative in tracked_contract_paths(contract_major):
        body = (ROOT / relative).read_bytes()
        entries.append(
            {
                "bytes": len(body),
                "path": relative.as_posix(),
                "sha256": hashlib.sha256(body).hexdigest(),
            }
        )
    payload: dict[str, Any] = {
        "contract_version": suite["suite_version"],
        "files": entries,
        "format": f"chill-contract-integrity-v{contract_major}",
    }
    if contract_major > 1:
        base_path = generated_manifest_path(1)
        base = load_json(base_path)
        payload["extends"] = {
            "contract_version": base["contract_version"],
            "manifest": base_path.relative_to(ROOT).as_posix(),
            "sha256": hashlib.sha256(base_path.read_bytes()).hexdigest(),
        }
    return payload


def validate_action_pins(workflow: Path, body: str) -> list[str]:
    errors: list[str] = []
    for number, line in enumerate(body.splitlines(), start=1):
        stripped = line.strip()
        if not stripped.startswith("uses:") and not stripped.startswith("- uses:"):
            continue
        normalized = stripped.removeprefix("- ")
        value = normalized.split(":", 1)[1].strip()
        if value.startswith("./"):
            continue
        candidate = line.replace("- uses:", "uses:", 1)
        if REMOTE_ACTION.match(candidate) is None:
            errors.append(
                f"{workflow.relative_to(ROOT)}:{number}: remote action must use "
                "a full 40-character commit with a version comment"
            )
    return errors


def validate_components() -> list[str]:
    errors: list[str] = []
    registry = load_json(ROOT / "ci" / "components.json")
    if registry.get("format") != "chill-component-registry-v1":
        errors.append("ci/components.json has an unsupported format")
        return errors
    seen: set[str] = set()
    for component in registry.get("components", []):
        identifier = component.get("id")
        if not isinstance(identifier, str) or identifier in seen:
            errors.append(f"component ID is missing or duplicated: {identifier!r}")
            continue
        seen.add(identifier)
        state = component.get("state")
        paths = component.get("paths")
        if state not in {"active", "planned"} or not isinstance(paths, list) or not paths:
            errors.append(f"component {identifier} has an invalid state or paths")
            continue
        if not component.get("owners") or not component.get("versioning"):
            errors.append(f"component {identifier} lacks ownership or versioning")
        if not component.get("release_artifact"):
            errors.append(f"component {identifier} lacks a release artifact contract")
        existing = [path for path in paths if (ROOT / path).exists()]
        if state == "active" and len(existing) != len(paths):
            errors.append(f"active component {identifier} has missing paths")
        if state == "planned":
            if not component.get("activation_ticket"):
                errors.append(f"planned component {identifier} lacks an activation ticket")
            if existing:
                errors.append(
                    f"planned component {identifier} now exists; activate its registry entry"
                )
    return errors


def validate_toolchains() -> list[str]:
    errors: list[str] = []
    toolchains = load_json(ROOT / "ci" / "toolchains.json")
    dockerfile = (ROOT / "backend" / "Dockerfile").read_text(encoding="utf-8")
    go_artifacts = [ROOT / "backend" / "go.mod", ROOT / "backend" / "go.sum"]
    go_artifacts.extend((ROOT / "backend").rglob("*.go"))
    if any(path.exists() for path in go_artifacts):
        errors.append("server-side Go source or module metadata remains")
    server_tooling = (
        (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
        + "\n"
        + dockerfile
    )
    if "setup-go" in server_tooling or "golang:" in server_tooling or re.search(
        r"\bgo\s+(?:test|build|vet|mod)\b", server_tooling
    ):
        errors.append("server build tooling still depends on Go")

    rust_version = toolchains["rust"]["version"]
    rust_toolchain = (ROOT / "backend" / "rust-toolchain.toml").read_text(
        encoding="utf-8"
    )
    cargo_manifest = (ROOT / "backend" / "Cargo.toml").read_text(encoding="utf-8")
    if f'channel = "{rust_version}"' not in rust_toolchain:
        errors.append("Rust toolchain manifest and backend/rust-toolchain.toml disagree")
    if f'rust-version = "{rust_version.removesuffix(".0")}"' not in cargo_manifest:
        errors.append("Rust toolchain manifest and backend/Cargo.toml disagree")
    if f"rust:{rust_version}-" not in dockerfile:
        errors.append("Rust toolchain manifest and backend/Dockerfile disagree")
    portable_manifest = (ROOT / "sdk" / "rust" / "Cargo.toml").read_text(
        encoding="utf-8"
    )
    if f'rust-version = "{rust_version.removesuffix(".0")}"' not in portable_manifest:
        errors.append("Rust toolchain manifest and portable Rust SDK disagree")

    javascript = load_json(ROOT / "sdk" / "js" / "package.json")
    if javascript.get("devDependencies", {}).get("typescript") != toolchains["node"][
        "web_typescript"
    ]:
        errors.append("TypeScript toolchain manifest and JavaScript SDK disagree")
    tauri_lock = (ROOT / "sdk" / "tauri" / "Cargo.lock").read_text(encoding="utf-8")
    tauri_version = toolchains["tauri"]["locked_version"]
    if f'name = "tauri"\nversion = "{tauri_version}"' not in tauri_lock:
        errors.append("Tauri toolchain manifest and lockfile disagree")

    swift_version = toolchains["swift"]["tools_version"]
    package = (ROOT / "sdk" / "swift" / "Package.swift").read_text(encoding="utf-8")
    if not package.startswith(f"// swift-tools-version: {swift_version}\n"):
        errors.append("Swift toolchain manifest and Package.swift disagree")
    image = toolchains["swift"]["linux_image"]
    if "@sha256:" not in image or len(image.rsplit("@sha256:", 1)[1]) != 64:
        errors.append("Swift Linux image must be pinned by SHA-256 digest")

    android = toolchains["android"]
    android_root = ROOT / "sdk" / "android"
    root_build = (android_root / "build.gradle.kts").read_text(encoding="utf-8")
    if f'id("com.android.library") version "{android["agp"]}"' not in root_build:
        errors.append("Android toolchain manifest and Gradle plugin version disagree")
    kotlin_version = android["kotlin"]
    for plugin in ("org.jetbrains.kotlin.jvm", "org.jetbrains.kotlin.plugin.compose"):
        if f'id("{plugin}") version "{kotlin_version}"' not in root_build:
            errors.append(
                f"Android toolchain manifest and {plugin} version disagree"
            )
    wrapper = (
        android_root / "gradle" / "wrapper" / "gradle-wrapper.properties"
    ).read_text(encoding="utf-8")
    if f"gradle-{android['gradle']}-bin.zip" not in wrapper:
        errors.append("Android toolchain manifest and Gradle wrapper disagree")
    if "distributionSha256Sum=" not in wrapper:
        errors.append("Gradle distribution must be pinned by SHA-256 checksum")
    for module in ("android", "samples"):
        build = (android_root / module / "build.gradle.kts").read_text(
            encoding="utf-8"
        )
        if f"compileSdk = {android['compile_sdk']}" not in build:
            errors.append(
                f"Android toolchain manifest and :{module} compileSdk disagree"
            )
        if f"minSdk = {android['min_sdk']}" not in build:
            errors.append(
                f"Android toolchain manifest and :{module} minSdk disagree"
            )
    return errors


def validate_workflows() -> list[str]:
    errors: list[str] = []
    workflows = sorted((ROOT / ".github" / "workflows").glob("*.yml"))
    if not workflows:
        return ["no GitHub Actions workflows found"]
    for workflow in workflows:
        body = workflow.read_text(encoding="utf-8")
        errors.extend(validate_action_pins(workflow, body))
        if "pull_request_target:" in body:
            errors.append(f"{workflow.relative_to(ROOT)} uses pull_request_target")
        if "permissions:" not in body:
            errors.append(f"{workflow.relative_to(ROOT)} lacks explicit permissions")
    continuous_integration = ROOT / ".github" / "workflows" / "ci.yml"
    if not continuous_integration.exists():
        errors.append("continuous integration workflow is missing")
    else:
        body = continuous_integration.read_text(encoding="utf-8")
        toolchains = load_json(ROOT / "ci" / "toolchains.json")
        required_gates = {
            "secret scanning": "gitleaks/gitleaks-action@",
            "repository contract": "scripts/repository_contract.py verify",
            "documentation": "scripts/validate_documentation.py",
            "backend integration": "scripts/test-backend-integration.sh",
            "Swift package contract": "scripts/validate-swift-package-linux.sh",
            "portable conformance": "scripts/run-portable-conformance.sh",
            "portable Rust SDK": "working-directory: sdk/rust",
            "portable JavaScript SDK": "working-directory: sdk/js",
            "Tauri SDK": "working-directory: sdk/tauri",
            "Unity SDK": "scripts/validate-unity-sdk.py",
            "OCI build": "docker/build-push-action@",
        }
        for gate, token in required_gates.items():
            if token not in body:
                errors.append(f"continuous integration lacks the {gate} gate")
        swift_image = toolchains["swift"]["linux_image"]
        if f"image: {swift_image}" not in body:
            errors.append("continuous integration does not use the pinned Swift image")
        expected_versions = (
            ("runner", f"runs-on: {toolchains['github']['linux_runner']}"),
            ("Python", f'python-version: "{toolchains["python"]["version"]}"'),
            ("Rust", f"rustup toolchain install {toolchains['rust']['version']}"),
            ("Deno", f'deno-version: "{toolchains["deno"]["version"]}"'),
            ("Bun", f'bun-version: "{toolchains["bun"]["version"]}"'),
        )
        for name, token in expected_versions:
            if token not in body:
                errors.append(f"continuous integration does not use the pinned {name}")
    release = ROOT / ".github" / "workflows" / "release.yml"
    if not release.exists():
        errors.append("release workflow is missing")
    else:
        body = release.read_text(encoding="utf-8")
        if body.count("actions/attest@") < 2 or "push-to-registry: true" not in body:
            errors.append("release workflow lacks signed artifact and OCI provenance")
    return errors


def validate_public_repository() -> list[str]:
    errors: list[str] = []
    for relative in COMMUNITY_FILES:
        if not (ROOT / relative).is_file():
            errors.append(f"missing public repository file: {relative}")

    license_path = ROOT / "LICENSE"
    if license_path.is_file() and "Apache License\n                           Version 2.0" not in license_path.read_text(
        encoding="utf-8"
    ):
        errors.append("LICENSE is not the Apache License 2.0 text")

    scope_events = subprocess.run(
        ["git", "ls-files", "--", ".scope/events"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    if scope_events:
        errors.append("machine-local Scope events must not be tracked")

    fly_config = (ROOT / "fly.toml").read_text(encoding="utf-8")
    for private_value in ("briannadoubt", ".chatgpt.site", "production-"):
        if private_value in fly_config:
            errors.append(f"fly.toml contains a deployment-specific value: {private_value}")

    for manifest in (
        ROOT / "backend" / "Cargo.toml",
        ROOT / "sdk" / "web" / "package.json",
        ROOT / "sdk" / "unity" / "package.json",
    ):
        if "Apache-2.0" not in manifest.read_text(encoding="utf-8"):
            errors.append(f"{manifest.relative_to(ROOT)} does not declare Apache-2.0")
    web_license = ROOT / "sdk" / "web" / "LICENSE"
    if web_license.is_file() and web_license.read_bytes() != license_path.read_bytes():
        errors.append("sdk/web/LICENSE differs from the repository license")
    for dockerfile in (ROOT / "backend" / "Dockerfile", ROOT / "console" / "Dockerfile"):
        if "/usr/local/share/licenses/chill/" not in dockerfile.read_text(encoding="utf-8"):
            errors.append(f"{dockerfile.relative_to(ROOT)} does not install license files")
    return errors


def validate_version(release_tag: str | None) -> list[str]:
    errors: list[str] = []
    version = (ROOT / "VERSION").read_text(encoding="utf-8").strip()
    if SEMVER.fullmatch(version) is None:
        errors.append("VERSION is not canonical SemVer")
    if release_tag is not None:
        if STABLE_SEMVER.fullmatch(version) is None:
            errors.append("a release tag requires a stable VERSION")
        if release_tag != f"v{version}":
            errors.append(f"release tag {release_tag!r} does not match VERSION {version!r}")
    return errors


def verify(release_tag: str | None) -> list[str]:
    errors: list[str] = []
    majors = sorted(
        int(path.parent.name.removeprefix("v"))
        for path in (ROOT / "conformance").glob("v*/manifest.json")
        if re.fullmatch(r"v[0-9]+", path.parent.name)
    )
    for contract_major in majors:
        generated = generated_manifest_path(contract_major)
        expected = canonical_json(manifest_payload(contract_major))
        if not generated.exists():
            errors.append(f"generated contract manifest is missing: {generated}")
        elif generated.read_bytes() != expected:
            errors.append(
                "generated contract manifest is stale; run "
                "python3 scripts/repository_contract.py update "
                f"--contract-major {contract_major}"
            )
    errors.extend(validate_components())
    errors.extend(validate_toolchains())
    errors.extend(validate_workflows())
    errors.extend(validate_public_repository())
    errors.extend(validate_version(release_tag))
    return errors


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    update_parser = subparsers.add_parser(
        "update", help="rewrite one deterministic contract manifest"
    )
    update_parser.add_argument(
        "--contract-major", type=int, default=1, choices=range(1, 100)
    )
    verify_parser = subparsers.add_parser("verify", help="verify repository invariants")
    verify_parser.add_argument("--release-tag", help="require a stable matching vX.Y.Z tag")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        if args.command == "update":
            generated = generated_manifest_path(args.contract_major)
            generated.parent.mkdir(parents=True, exist_ok=True)
            generated.write_bytes(canonical_json(manifest_payload(args.contract_major)))
            print(generated.relative_to(ROOT))
            return 0
        errors = verify(args.release_tag)
    except (OSError, KeyError, TypeError, json.JSONDecodeError, subprocess.CalledProcessError, RepositoryContractError) as error:
        print(f"repository contract failed: {error}", file=sys.stderr)
        return 2
    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 1
    print("repository-contract-ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
