# Portable SDK V2 validation

`v2/scenarios.json` defines executable evidence templates for every portable SDK profile. `local_presubmit` scenarios are deterministic smoke checks: they catch missing instrumentation and schema regressions but are not release evidence. `release_evidence` scenarios require a signed release build, paired compiled-out control, the documented floor environment, and fresh evidence accepted by `contracts.budgets.v2.evaluate_report`.

No template may be substituted for a release report; the V2 catalog's seven-day freshness limit is enforced by the evaluator.
