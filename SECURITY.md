# Security policy

Chill handles telemetry, credentials, tenant data, and privacy-sensitive
workflows. Please report suspected vulnerabilities privately and give the
maintainer a reasonable opportunity to investigate before public disclosure.

## Supported versions

Chill is currently pre-1.0. Security fixes are applied to the latest release
and the default branch only. Older snapshots are not supported.

| Version | Supported |
| --- | --- |
| Latest 0.1.x release | Yes |
| Default branch | Best effort |
| Older versions | No |

## Report a vulnerability

Use GitHub's **Report a vulnerability** action in this repository's Security
tab. Do not open a public issue or include secrets, tenant data, or exploit
details in a discussion.

Include the affected revision, component, deployment assumptions, impact, and
minimal reproduction steps. Reports are acknowledged as soon as practical.
The maintainer will coordinate validation, remediation, release, and credit
with the reporter. No response-time or bounty commitment is currently offered.

## Scope

Reports about the Rust backend, SDKs, console, deployment assets, contracts,
and repository automation are in scope. Vulnerabilities in third-party hosted
services should be reported to that provider unless Chill's configuration or
integration is the cause.

Testing must use systems and data you own or are explicitly authorized to test.
Do not degrade service, access other tenants, retain personal data, or expose a
vulnerability beyond what is necessary to demonstrate it.
