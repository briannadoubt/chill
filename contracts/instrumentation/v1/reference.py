"""Executable semantics for declarative actions and macro instrumentation.

The model is intentionally framework-neutral and immutable. Platform runtimes
may use locks, actors, atomics, task locals, ring buffers, and generated static
descriptors as long as their emitted facts are observationally equivalent.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from enum import StrEnum
import re


_NAME_PATTERN = re.compile(r"^[a-z][a-z0-9_]*(?:[.-][a-z0-9_]+)*$")
_DIGEST_PATTERN = re.compile(r"^[0-9a-f]{64}$")
_ACTIVITY_KINDS = {"ui", "domain", "network", "storage", "task", "custom"}
_ACTION_ACTIVATIONS = {
    "primary",
    "submit",
    "toggle",
    "selection",
    "adjust",
    "gesture",
    "system",
}
_EVENT_CLASSES = {
    "lifecycle",
    "domain",
    "error",
    "crash",
    "performance",
    "experiment",
    "custom",
}
_EVENT_SEVERITIES = {"trace", "debug", "info", "warn", "error", "fatal"}


class ActivityRole(StrEnum):
    OPERATION = "operation"
    ATTEMPT = "attempt"


class TerminalOutcome(StrEnum):
    SUCCEEDED = "succeeded"
    FAILED = "failed"
    CANCELLED = "cancelled"
    TIMED_OUT = "timed_out"


class EventEmission(StrEnum):
    ENTERED = "entered"
    SUCCEEDED = "succeeded"
    TERMINAL = "terminal"


class InputKind(StrEnum):
    TOUCH = "touch"
    POINTER = "pointer"
    KEYBOARD = "keyboard"
    REMOTE = "remote"
    ACCESSIBILITY = "accessibility"
    VOICE = "voice"
    SYSTEM = "system"
    UNKNOWN = "unknown"


class _ActivityPhase(StrEnum):
    ACTIVE = "active"
    SUCCEEDED = "succeeded"
    FAILED = "failed"
    CANCELLED = "cancelled"
    TIMED_OUT = "timed_out"


class _EventPhase(StrEnum):
    ACTIVE = "active"
    COMPLETED = "completed"


@dataclass(frozen=True)
class ActivityDefinition:
    name: str
    kind: str = "domain"
    role: ActivityRole = ActivityRole.OPERATION

    def __post_init__(self) -> None:
        _validate_name(self.name, "activity name")
        if self.kind not in _ACTIVITY_KINDS:
            raise ValueError(f"invalid activity kind: {self.kind}")


@dataclass(frozen=True)
class _ActivityInstance:
    instance_id: str
    definition: ActivityDefinition
    parent_activity_id: str | None
    started_monotonic_nano: int
    attempt: int
    recursion_depth: int
    phase: _ActivityPhase = _ActivityPhase.ACTIVE


@dataclass(frozen=True)
class ActivityFact:
    operation: str
    subject_id: str
    name: str
    kind: str
    role: ActivityRole
    parent_activity_id: str | None
    attempt: int
    recursion_depth: int
    outcome: TerminalOutcome | None = None
    duration_nano: int | None = None
    reason_code: str | None = None


@dataclass(frozen=True)
class NativeActivation:
    native_activation_id: str
    record_id: str
    surface_id: str
    element_instance_id: str
    name: str
    role: str
    activation: str
    input_kind: InputKind

    def __post_init__(self) -> None:
        if not self.native_activation_id or not self.record_id:
            raise ValueError("activation and record IDs must not be empty")
        if not self.surface_id or not self.element_instance_id:
            raise ValueError("surface and element instance IDs must not be empty")
        _validate_name(self.name, "action name")
        _validate_name(self.role, "action role")
        if self.activation not in _ACTION_ACTIVATIONS:
            raise ValueError(f"invalid activation kind: {self.activation}")


@dataclass(frozen=True)
class ActionFact:
    record_id: str
    name: str
    role: str
    activation: str
    input_kind: InputKind
    surface_id: str
    element_instance_id: str


@dataclass(frozen=True)
class EventDefinition:
    name: str
    event_class: str = "domain"
    severity: str = "info"
    emission: EventEmission = EventEmission.SUCCEEDED

    def __post_init__(self) -> None:
        _validate_name(self.name, "event name")
        if self.event_class not in _EVENT_CLASSES:
            raise ValueError(f"invalid event class: {self.event_class}")
        if self.severity not in _EVENT_SEVERITIES:
            raise ValueError(f"invalid event severity: {self.severity}")


@dataclass(frozen=True)
class _EventInvocation:
    invocation_id: str
    record_id: str
    definition: EventDefinition
    deduplication_key_hash: str | None
    phase: _EventPhase = _EventPhase.ACTIVE
    emitted: bool = False


@dataclass(frozen=True)
class EventFact:
    record_id: str
    name: str
    event_class: str
    severity: str
    emission: EventEmission
    outcome: TerminalOutcome | None
    deduplication_key_hash: str | None


@dataclass(frozen=True)
class RuntimeState:
    activities: tuple[_ActivityInstance, ...] = ()
    retry_counters: tuple[tuple[str, str, int], ...] = ()
    seen_activation_ids: tuple[tuple[str, str], ...] = ()
    event_invocations: tuple[_EventInvocation, ...] = ()
    seen_event_keys: frozenset[tuple[str, str]] = frozenset()

    def __post_init__(self) -> None:
        activity_ids = [item.instance_id for item in self.activities]
        if len(activity_ids) != len(set(activity_ids)):
            raise ValueError("duplicate activity instance")
        invocation_ids = [item.invocation_id for item in self.event_invocations]
        if len(invocation_ids) != len(set(invocation_ids)):
            raise ValueError("duplicate event invocation")

    @property
    def activities_by_id(self) -> dict[str, _ActivityInstance]:
        return {item.instance_id: item for item in self.activities}

    @property
    def events_by_id(self) -> dict[str, _EventInvocation]:
        return {item.invocation_id: item for item in self.event_invocations}


@dataclass(frozen=True)
class ActivityTransition:
    state: RuntimeState
    facts: tuple[ActivityFact, ...] = ()
    diagnostics: tuple[str, ...] = ()


@dataclass(frozen=True)
class ActionTransition:
    state: RuntimeState
    facts: tuple[ActionFact, ...] = ()
    diagnostics: tuple[str, ...] = ()


@dataclass(frozen=True)
class EventTransition:
    state: RuntimeState
    facts: tuple[EventFact, ...] = ()
    diagnostics: tuple[str, ...] = ()


def _validate_name(value: str, label: str) -> None:
    if not _NAME_PATTERN.fullmatch(value):
        raise ValueError(f"invalid {label}: {value}")


def _replace_activity(
    state: RuntimeState, replacement: _ActivityInstance
) -> RuntimeState:
    return replace(
        state,
        activities=tuple(
            replacement if item.instance_id == replacement.instance_id else item
            for item in state.activities
        ),
    )


def _replace_event(state: RuntimeState, replacement: _EventInvocation) -> RuntimeState:
    return replace(
        state,
        event_invocations=tuple(
            replacement if item.invocation_id == replacement.invocation_id else item
            for item in state.event_invocations
        ),
    )


def _ancestor_instances(
    state: RuntimeState, parent_activity_id: str | None
) -> tuple[_ActivityInstance, ...]:
    by_id = state.activities_by_id
    result: list[_ActivityInstance] = []
    cursor_id = parent_activity_id
    seen: set[str] = set()
    while cursor_id is not None:
        if cursor_id in seen:
            raise ValueError("activity ancestry contains a cycle")
        seen.add(cursor_id)
        try:
            cursor = by_id[cursor_id]
        except KeyError as error:
            raise ValueError(f"unknown parent activity: {cursor_id}") from error
        result.append(cursor)
        cursor_id = cursor.parent_activity_id
    return tuple(result)


def begin_activity(
    state: RuntimeState,
    definition: ActivityDefinition,
    instance_id: str,
    started_monotonic_nano: int,
    *,
    parent_activity_id: str | None = None,
) -> ActivityTransition:
    """Begin one macro invocation and emit exactly one start fact."""

    if not instance_id:
        raise ValueError("activity instance ID must not be empty")
    if started_monotonic_nano < 0:
        raise ValueError("monotonic time must not be negative")
    if instance_id in state.activities_by_id:
        return ActivityTransition(state, diagnostics=("activity.duplicate_start",))

    ancestors = _ancestor_instances(state, parent_activity_id)
    recursion_depth = sum(
        ancestor.definition.name == definition.name for ancestor in ancestors
    )

    retry_counters = dict(
        ((parent_id, name), count)
        for parent_id, name, count in state.retry_counters
    )
    attempt = 1
    if definition.role is ActivityRole.ATTEMPT:
        if parent_activity_id is None:
            raise ValueError("a retry attempt requires a parent activity")
        key = (parent_activity_id, definition.name)
        attempt = retry_counters.get(key, 0) + 1
        retry_counters[key] = attempt

    instance = _ActivityInstance(
        instance_id=instance_id,
        definition=definition,
        parent_activity_id=parent_activity_id,
        started_monotonic_nano=started_monotonic_nano,
        attempt=attempt,
        recursion_depth=recursion_depth,
    )
    next_state = replace(
        state,
        activities=state.activities + (instance,),
        retry_counters=tuple(
            (parent_id, name, count)
            for (parent_id, name), count in sorted(retry_counters.items())
        ),
    )
    fact = ActivityFact(
        operation="start",
        subject_id=instance_id,
        name=definition.name,
        kind=definition.kind,
        role=definition.role,
        parent_activity_id=parent_activity_id,
        attempt=attempt,
        recursion_depth=recursion_depth,
    )
    return ActivityTransition(next_state, facts=(fact,))


def finish_activity(
    state: RuntimeState,
    instance_id: str,
    ended_monotonic_nano: int,
    outcome: TerminalOutcome,
    *,
    reason_code: str | None = None,
) -> ActivityTransition:
    """Finish one invocation; repeated terminal callbacks emit no second end."""

    try:
        instance = state.activities_by_id[instance_id]
    except KeyError:
        return ActivityTransition(state, diagnostics=("activity.unknown_terminal",))
    if instance.phase is not _ActivityPhase.ACTIVE:
        return ActivityTransition(state, diagnostics=("activity.duplicate_terminal",))
    if ended_monotonic_nano < instance.started_monotonic_nano:
        raise ValueError("activity end precedes start")
    if reason_code is not None:
        _validate_name(reason_code, "reason code")

    terminal_phase = {
        TerminalOutcome.SUCCEEDED: _ActivityPhase.SUCCEEDED,
        TerminalOutcome.FAILED: _ActivityPhase.FAILED,
        TerminalOutcome.CANCELLED: _ActivityPhase.CANCELLED,
        TerminalOutcome.TIMED_OUT: _ActivityPhase.TIMED_OUT,
    }[outcome]
    finished = replace(instance, phase=terminal_phase)
    next_state = _replace_activity(state, finished)
    fact = ActivityFact(
        operation="end",
        subject_id=instance.instance_id,
        name=instance.definition.name,
        kind=instance.definition.kind,
        role=instance.definition.role,
        parent_activity_id=instance.parent_activity_id,
        attempt=instance.attempt,
        recursion_depth=instance.recursion_depth,
        outcome=outcome,
        duration_nano=ended_monotonic_nano - instance.started_monotonic_nano,
        reason_code=reason_code,
    )
    return ActivityTransition(next_state, facts=(fact,))


def observe_action(
    state: RuntimeState,
    activation: NativeActivation,
    *,
    deduplication_capacity: int = 256,
) -> ActionTransition:
    """Normalize many observer callbacks into one semantic native activation."""

    if deduplication_capacity < 1:
        raise ValueError("deduplication capacity must be positive")
    key = (activation.surface_id, activation.native_activation_id)
    if key in state.seen_activation_ids:
        return ActionTransition(state, diagnostics=("action.duplicate_observation",))

    seen = (state.seen_activation_ids + (key,))[-deduplication_capacity:]
    next_state = replace(state, seen_activation_ids=seen)
    fact = ActionFact(
        record_id=activation.record_id,
        name=activation.name,
        role=activation.role,
        activation=activation.activation,
        input_kind=activation.input_kind,
        surface_id=activation.surface_id,
        element_instance_id=activation.element_instance_id,
    )
    return ActionTransition(next_state, facts=(fact,))


def _emit_event(
    state: RuntimeState,
    invocation: _EventInvocation,
    outcome: TerminalOutcome | None,
) -> EventTransition:
    key_hash = invocation.deduplication_key_hash
    semantic_key = (
        (invocation.definition.name, key_hash) if key_hash is not None else None
    )
    emitted_invocation = replace(invocation, emitted=True)
    state = _replace_event(state, emitted_invocation)
    if semantic_key is not None and semantic_key in state.seen_event_keys:
        return EventTransition(state, diagnostics=("event.semantic_duplicate",))

    seen = state.seen_event_keys
    if semantic_key is not None:
        seen = seen | {semantic_key}
        state = replace(state, seen_event_keys=seen)
    fact = EventFact(
        record_id=invocation.record_id,
        name=invocation.definition.name,
        event_class=invocation.definition.event_class,
        severity=invocation.definition.severity,
        emission=invocation.definition.emission,
        outcome=outcome,
        deduplication_key_hash=key_hash,
    )
    return EventTransition(state, facts=(fact,))


def begin_event(
    state: RuntimeState,
    definition: EventDefinition,
    invocation_id: str,
    record_id: str,
    *,
    deduplication_key_hash: str | None = None,
) -> EventTransition:
    """Begin a macro invocation and emit immediately only for ENTERED events."""

    if not invocation_id or not record_id:
        raise ValueError("event invocation and record IDs must not be empty")
    if invocation_id in state.events_by_id:
        return EventTransition(state, diagnostics=("event.duplicate_start",))
    if deduplication_key_hash is not None and not _DIGEST_PATTERN.fullmatch(
        deduplication_key_hash
    ):
        raise ValueError("event deduplication key must be a lowercase SHA-256 digest")

    invocation = _EventInvocation(
        invocation_id=invocation_id,
        record_id=record_id,
        definition=definition,
        deduplication_key_hash=deduplication_key_hash,
    )
    next_state = replace(
        state, event_invocations=state.event_invocations + (invocation,)
    )
    if definition.emission is EventEmission.ENTERED:
        return _emit_event(next_state, invocation, None)
    return EventTransition(next_state)


def finish_event(
    state: RuntimeState,
    invocation_id: str,
    outcome: TerminalOutcome,
) -> EventTransition:
    """Finish a macro invocation, preserving its declared emission policy."""

    try:
        invocation = state.events_by_id[invocation_id]
    except KeyError:
        return EventTransition(state, diagnostics=("event.unknown_terminal",))
    if invocation.phase is _EventPhase.COMPLETED:
        return EventTransition(state, diagnostics=("event.duplicate_terminal",))

    should_emit = (
        invocation.definition.emission is EventEmission.TERMINAL
        or (
            invocation.definition.emission is EventEmission.SUCCEEDED
            and outcome is TerminalOutcome.SUCCEEDED
        )
    )
    result = EventTransition(state)
    if should_emit and not invocation.emitted:
        result = _emit_event(state, invocation, outcome)

    completed = replace(
        result.state.events_by_id[invocation_id], phase=_EventPhase.COMPLETED
    )
    completed_state = _replace_event(result.state, completed)
    return EventTransition(completed_state, result.facts, result.diagnostics)
