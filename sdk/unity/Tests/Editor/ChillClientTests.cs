using System;
using System.IO;
using System.Linq;
using NUnit.Framework;

namespace Chill.Unity.Tests
{
    public sealed class ChillClientTests
    {
        [Test]
        public void DeniedConsentPerformsNoCaptureWork()
        {
            var store = new ChillMemoryStore();
            ChillClient client = Client(ChillConsent.Denied, store);

            client.Start();
            Assert.That(client.Event("game.started"), Is.False);
            Assert.That(store.Peek(10), Is.Empty);
        }

        [Test]
        public void GrantStartsSessionAndDenialPurgesQueue()
        {
            var store = new ChillMemoryStore();
            ChillClient client = Client(ChillConsent.Denied, store);
            client.Start();

            client.SetConsent(ChillConsent.Granted);
            Assert.That(store.Peek(10).Count, Is.EqualTo(1));
            Assert.That(store.Peek(10)[0].LogJson, Does.Contain("\"chill.behavior.kind\""));
            Assert.That(store.Peek(10)[0].LogJson, Does.Contain("\"session\""));

            client.SetConsent(ChillConsent.Denied);
            Assert.That(store.Peek(10), Is.Empty);
        }

        [Test]
        public void AnnotationsAreAllowlistedAndClassified()
        {
            var store = new ChillMemoryStore();
            ChillClient client = Client(ChillConsent.Granted, store);
            client.DeclareAnnotation(
                "player.cohort",
                ChillAnnotationClassification.PseudonymousIdentifier);
            client.Start();

            ChillAnnotations values = new ChillAnnotations()
                .With("player.cohort", "returning")
                .With("private.raw", "privacy-canary");
            Assert.That(client.Action("store.purchase", values), Is.True);

            string log = store.Peek(10).Last().LogJson;
            Assert.That(log, Does.Contain("chill.annotation.player.cohort"));
            Assert.That(log, Does.Contain("pseudonymous_identifier"));
            Assert.That(log, Does.Not.Contain("privacy-canary"));
            Assert.That(
                client.Diagnostics.Any(value => value.Code == "annotation_disallowed"),
                Is.True);
        }

        [Test]
        public void OuterAnnotationsWinCollisions()
        {
            ChillAnnotations outer = new ChillAnnotations().With("match.mode", "ranked");
            ChillAnnotations inner = new ChillAnnotations()
                .With("match.mode", "private")
                .With("level.id", "cave");

            ChillAnnotations merged = outer.MergeDescendant(inner);

            Assert.That(merged.Entries.Count, Is.EqualTo(2));
            Assert.That(merged.Entries[0].Key, Is.EqualTo("match.mode"));
            Assert.That(merged.Entries[0].Value.Value, Is.EqualTo("ranked"));
        }

        [Test]
        public void ActivityProducesPairedTerminalRecordsWithoutErrorText()
        {
            var store = new ChillMemoryStore();
            ChillClient client = Client(ChillConsent.Granted, store);
            client.Start();

            ChillActivityScope activity = client.BeginActivity(
                "inventory.load",
                null,
                ChillActivityKind.Storage);
            activity.Fail("cache_miss");

            string[] logs = store.Peek(10).Select(value => value.LogJson).ToArray();
            Assert.That(logs.Count(value => value.Contains("\"activity\"")), Is.EqualTo(2));
            Assert.That(logs.Last(), Does.Contain("cache_miss"));
            Assert.That(logs.Last(), Does.Contain("\"error\""));
        }

        [Test]
        public void PageScopeCarriesStructuredContext()
        {
            var store = new ChillMemoryStore();
            ChillClient client = Client(ChillConsent.Granted, store);
            client.Start();

            ChillPageScope page = client.StartPage(
                "main_menu",
                null,
                ChillPageRelation.Root,
                ChillPageCause.Initial);
            client.Action("play.start");
            page.Dispose();

            string[] logs = store.Peek(10).Select(value => value.LogJson).ToArray();
            Assert.That(logs.Any(value => value.Contains("chill.payload.path")), Is.True);
            Assert.That(logs.Any(value => value.Contains("chill.context.page.path")), Is.True);
            Assert.That(logs.Last(), Does.Contain("\"end\""));
        }

        [Test]
        public void ExportAcknowledgementIsAtLeastOnce()
        {
            var store = new ChillMemoryStore();
            ChillClient client = Client(ChillConsent.Granted, store);
            client.Start();
            client.Event("match.started");

            ChillExportBatch first = client.CreateExportBatch();
            ChillExportBatch retry = client.CreateExportBatch();

            Assert.That(first.Records.Select(value => value.RecordId), Is.EqualTo(
                retry.Records.Select(value => value.RecordId)));
            Assert.That(first.Body, Does.Contain("\"resourceLogs\""));
            Assert.That(first.Body, Does.Contain("\"process.runtime.name\""));
            Assert.That(first.Body, Does.Contain("\"chill.clock.monotonic_nano\""));
            Assert.That(first.Body, Does.Contain("\"chill.clock.boot_id\""));
            string fixturePath = Environment.GetEnvironmentVariable("CHILL_UNITY_OTLP_FIXTURE");
            if (!string.IsNullOrEmpty(fixturePath))
            {
                System.IO.File.WriteAllText(fixturePath, first.Body);
            }
            client.Acknowledge(first);
            Assert.That(store.Peek(10), Is.Empty);
        }

        [Test]
        public void IdentifiersUseCanonicalVersions()
        {
            string v4 = ChillIds.NewUuidV4();
            string v7 = ChillIds.NewUuidV7(1700000000000L);

            Assert.That(ChillIds.IsUuidV4(v4), Is.True);
            Assert.That(v7[14], Is.EqualTo('7'));
            Assert.That(v7, Has.Length.EqualTo(36));
        }

        [Test]
        public void EndpointMustBeExactAndSecure()
        {
            Assert.Throws<ArgumentException>(() =>
                new ChillConfiguration(
                    "test.game",
                    new Uri("http://telemetry.example.com/v1/logs"),
                    "sdk"));
            Assert.Throws<ArgumentException>(() =>
                new ChillConfiguration(
                    "test.game",
                    new Uri("https://telemetry.example.com/v1/logs?secret=value"),
                    "sdk"));
            Assert.DoesNotThrow(() =>
                new ChillConfiguration(
                    "test.game",
                    new Uri("http://localhost:4318/v1/logs"),
                    "sdk"));
        }

        [Test]
        public void FileStorePurgeRemovesQueuedTemporaryAndQuarantinedData()
        {
            string directory = Path.Combine(
                Path.GetTempPath(),
                "chill-unity-store-" + Guid.NewGuid().ToString("N"));
            try
            {
                var store = new ChillFileStore(directory, 1024 * 1024, delegate { });
                File.WriteAllText(Path.Combine(directory, "record.json"), "invalid");
                store.Peek(10);
                File.WriteAllText(Path.Combine(directory, ".pending.tmp"), "private");

                store.Purge();

                Assert.That(Directory.GetFiles(directory), Is.Empty);
            }
            finally
            {
                if (Directory.Exists(directory))
                {
                    Directory.Delete(directory, true);
                }
            }
        }

        [Test]
        public void ActivityScopeCanOutliveClientShutdown()
        {
            var store = new ChillMemoryStore();
            ChillClient client = Client(ChillConsent.Granted, store);
            client.Start();
            ChillActivityScope activity = client.BeginActivity("inventory.load");

            client.Dispose();

            Assert.DoesNotThrow(() => activity.Dispose());
        }

        private static ChillClient Client(ChillConsent consent, IChillStore store)
        {
            var configuration = new ChillConfiguration(
                "test.game",
                new Uri("https://telemetry.example.com/v1/logs"),
                "sdk-key")
            {
                Consent = consent,
                InstallationId = "10203040-5060-4080-9010-203040506070"
            };
            return new ChillClient(
                configuration,
                new FixedClock(),
                store,
                "server",
                "6000.5.4f1",
                "macos");
        }

        private sealed class FixedClock : IChillClock
        {
            public long UnixMilliseconds
            {
                get { return 1700000000000L; }
            }

            public string WallUnixNanoseconds
            {
                get { return "1700000000000000000"; }
            }

            public string MonotonicNanoseconds
            {
                get { return "1000000"; }
            }
        }
    }
}
