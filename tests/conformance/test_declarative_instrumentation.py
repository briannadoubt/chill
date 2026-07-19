from __future__ import annotations

import unittest

from contracts.instrumentation.v1 import (
    ActivityDefinition,
    ActivityRole,
    EventDefinition,
    EventEmission,
    InputKind,
    NativeActivation,
    RuntimeState,
    TerminalOutcome,
    begin_activity,
    begin_event,
    finish_activity,
    finish_event,
    observe_action,
)


class DeclarativeActionExamples(unittest.TestCase):
    def activation(self, native_id: str, record_id: str = "record-1") -> NativeActivation:
        return NativeActivation(
            native_activation_id=native_id,
            record_id=record_id,
            surface_id="surface-1",
            element_instance_id="button-instance-1",
            name="cat.adopt",
            role="button",
            activation="primary",
            input_kind=InputKind.TOUCH,
        )

    def test_rendering_a_declaration_emits_nothing(self) -> None:
        self.assertEqual(RuntimeState().seen_activation_ids, ())

    def test_observer_layers_collapse_to_one_native_activation(self) -> None:
        first = observe_action(RuntimeState(), self.activation("native-1"))
        duplicate = observe_action(first.state, self.activation("native-1", "record-2"))

        self.assertEqual(len(first.facts), 1)
        self.assertEqual(first.facts[0].name, "cat.adopt")
        self.assertEqual(duplicate.facts, ())
        self.assertEqual(duplicate.diagnostics, ("action.duplicate_observation",))

    def test_repeated_user_activations_remain_distinct(self) -> None:
        first = observe_action(RuntimeState(), self.activation("native-1"))
        second = observe_action(first.state, self.activation("native-2", "record-2"))

        self.assertEqual(
            tuple(fact.record_id for fact in first.facts + second.facts),
            ("record-1", "record-2"),
        )

    def test_unknown_activation_kind_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "invalid activation kind"):
            NativeActivation(
                native_activation_id="native-1",
                record_id="record-1",
                surface_id="surface-1",
                element_instance_id="element-1",
                name="cat.adopt",
                role="button",
                activation="clicked",
                input_kind=InputKind.POINTER,
            )


class ActivityMacroExamples(unittest.TestCase):
    def test_sync_or_async_success_has_one_start_and_one_end(self) -> None:
        definition = ActivityDefinition("adoption.submit")
        started = begin_activity(RuntimeState(), definition, "activity-1", 100)
        ended = finish_activity(
            started.state,
            "activity-1",
            160,
            TerminalOutcome.SUCCEEDED,
        )

        self.assertEqual(started.facts[0].operation, "start")
        self.assertEqual(ended.facts[0].operation, "end")
        self.assertEqual(ended.facts[0].duration_nano, 60)
        self.assertEqual(ended.facts[0].outcome, TerminalOutcome.SUCCEEDED)

    def test_duplicate_activity_start_is_idempotent(self) -> None:
        definition = ActivityDefinition("adoption.submit")
        first = begin_activity(RuntimeState(), definition, "activity-1", 100)
        duplicate = begin_activity(first.state, definition, "activity-1", 100)

        self.assertEqual(duplicate.facts, ())
        self.assertEqual(duplicate.diagnostics, ("activity.duplicate_start",))

    def test_errors_and_cancellation_are_terminal_and_idempotent(self) -> None:
        definition = ActivityDefinition("adoption.submit")
        started = begin_activity(RuntimeState(), definition, "activity-1", 100)
        failed = finish_activity(
            started.state,
            "activity-1",
            120,
            TerminalOutcome.FAILED,
            reason_code="server.rejected",
        )
        duplicate = finish_activity(
            failed.state,
            "activity-1",
            130,
            TerminalOutcome.CANCELLED,
        )

        self.assertEqual(failed.facts[0].reason_code, "server.rejected")
        self.assertEqual(duplicate.facts, ())
        self.assertEqual(duplicate.diagnostics, ("activity.duplicate_terminal",))

    def test_cancellation_is_not_reclassified_as_failure(self) -> None:
        started = begin_activity(
            RuntimeState(), ActivityDefinition("search.load"), "activity-1", 1
        )
        cancelled = finish_activity(
            started.state,
            "activity-1",
            2,
            TerminalOutcome.CANCELLED,
        )

        self.assertEqual(cancelled.facts[0].outcome, TerminalOutcome.CANCELLED)

    def test_recursion_creates_nested_instances_with_depth(self) -> None:
        definition = ActivityDefinition("tree.walk")
        outer = begin_activity(RuntimeState(), definition, "outer", 1)
        inner = begin_activity(
            outer.state,
            definition,
            "inner",
            2,
            parent_activity_id="outer",
        )
        deepest = begin_activity(
            inner.state,
            definition,
            "deepest",
            3,
            parent_activity_id="inner",
        )

        self.assertEqual(outer.facts[0].recursion_depth, 0)
        self.assertEqual(inner.facts[0].recursion_depth, 1)
        self.assertEqual(deepest.facts[0].recursion_depth, 2)

    def test_retry_attempts_are_numbered_inside_a_coordinator_activity(self) -> None:
        coordinator = begin_activity(
            RuntimeState(), ActivityDefinition("cat.load"), "operation", 1
        )
        attempt_definition = ActivityDefinition(
            "cat.load.attempt", role=ActivityRole.ATTEMPT
        )
        first = begin_activity(
            coordinator.state,
            attempt_definition,
            "attempt-1",
            2,
            parent_activity_id="operation",
        )
        first_end = finish_activity(
            first.state,
            "attempt-1",
            3,
            TerminalOutcome.FAILED,
        )
        second = begin_activity(
            first_end.state,
            attempt_definition,
            "attempt-2",
            4,
            parent_activity_id="operation",
        )

        self.assertEqual(first.facts[0].attempt, 1)
        self.assertEqual(second.facts[0].attempt, 2)

    def test_retry_attempt_without_coordinator_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "requires a parent"):
            begin_activity(
                RuntimeState(),
                ActivityDefinition("request.attempt", role=ActivityRole.ATTEMPT),
                "attempt-1",
                1,
            )

    def test_unknown_activity_kind_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "invalid activity kind"):
            ActivityDefinition("cat.load", kind="database_query")

    def test_child_may_finish_after_its_parent_without_losing_ancestry(self) -> None:
        parent = begin_activity(
            RuntimeState(), ActivityDefinition("parent"), "parent", 1
        )
        child = begin_activity(
            parent.state,
            ActivityDefinition("child"),
            "child",
            2,
            parent_activity_id="parent",
        )
        parent_end = finish_activity(
            child.state, "parent", 3, TerminalOutcome.SUCCEEDED
        )
        child_end = finish_activity(
            parent_end.state, "child", 4, TerminalOutcome.SUCCEEDED
        )

        self.assertEqual(child_end.facts[0].parent_activity_id, "parent")


class EventMacroExamples(unittest.TestCase):
    HASH = "a" * 64

    def test_default_event_emits_only_after_success(self) -> None:
        definition = EventDefinition("adoption.confirmed")
        started = begin_event(RuntimeState(), definition, "invocation-1", "record-1")
        succeeded = finish_event(
            started.state, "invocation-1", TerminalOutcome.SUCCEEDED
        )

        self.assertEqual(started.facts, ())
        self.assertEqual(len(succeeded.facts), 1)
        self.assertEqual(succeeded.facts[0].outcome, TerminalOutcome.SUCCEEDED)

    def test_default_event_does_not_emit_for_error_or_cancellation(self) -> None:
        for outcome in (TerminalOutcome.FAILED, TerminalOutcome.CANCELLED):
            with self.subTest(outcome=outcome):
                started = begin_event(
                    RuntimeState(),
                    EventDefinition("adoption.confirmed"),
                    "invocation-1",
                    "record-1",
                )
                ended = finish_event(started.state, "invocation-1", outcome)
                self.assertEqual(ended.facts, ())

    def test_entered_event_emits_before_the_body_and_never_twice(self) -> None:
        started = begin_event(
            RuntimeState(),
            EventDefinition("auth.attempted", emission=EventEmission.ENTERED),
            "invocation-1",
            "record-1",
        )
        ended = finish_event(
            started.state, "invocation-1", TerminalOutcome.FAILED
        )

        self.assertEqual(len(started.facts), 1)
        self.assertIsNone(started.facts[0].outcome)
        self.assertEqual(ended.facts, ())

    def test_terminal_event_emits_the_actual_outcome(self) -> None:
        started = begin_event(
            RuntimeState(),
            EventDefinition("sync.finished", emission=EventEmission.TERMINAL),
            "invocation-1",
            "record-1",
        )
        ended = finish_event(
            started.state, "invocation-1", TerminalOutcome.TIMED_OUT
        )

        self.assertEqual(ended.facts[0].outcome, TerminalOutcome.TIMED_OUT)

    def test_semantic_idempotency_suppresses_repeated_invocations(self) -> None:
        definition = EventDefinition("message.received")
        first = begin_event(
            RuntimeState(),
            definition,
            "invocation-1",
            "record-1",
            deduplication_key_hash=self.HASH,
        )
        first_end = finish_event(
            first.state, "invocation-1", TerminalOutcome.SUCCEEDED
        )
        second = begin_event(
            first_end.state,
            definition,
            "invocation-2",
            "record-2",
            deduplication_key_hash=self.HASH,
        )
        second_end = finish_event(
            second.state, "invocation-2", TerminalOutcome.SUCCEEDED
        )

        self.assertEqual(len(first_end.facts), 1)
        self.assertEqual(second_end.facts, ())
        self.assertEqual(second_end.diagnostics, ("event.semantic_duplicate",))

    def test_duplicate_terminal_callback_never_emits_an_event_twice(self) -> None:
        started = begin_event(
            RuntimeState(),
            EventDefinition("adoption.confirmed"),
            "invocation-1",
            "record-1",
        )
        first = finish_event(
            started.state, "invocation-1", TerminalOutcome.SUCCEEDED
        )
        duplicate = finish_event(
            first.state, "invocation-1", TerminalOutcome.SUCCEEDED
        )

        self.assertEqual(len(first.facts), 1)
        self.assertEqual(duplicate.facts, ())
        self.assertEqual(duplicate.diagnostics, ("event.duplicate_terminal",))

    def test_invalid_event_definition_and_digest_are_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "invalid event severity"):
            EventDefinition("adoption.confirmed", severity="notice")
        with self.assertRaisesRegex(ValueError, "lowercase SHA-256"):
            begin_event(
                RuntimeState(),
                EventDefinition("adoption.confirmed"),
                "invocation-1",
                "record-1",
                deduplication_key_hash="not-a-digest",
            )


if __name__ == "__main__":
    unittest.main()
