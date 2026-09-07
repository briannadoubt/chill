using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Text;

namespace Chill.Unity
{
    internal sealed class ChillQueuedRecord
    {
        internal ChillQueuedRecord(
            string recordId,
            long sequence,
            string occurredAtUnixNanoseconds,
            string logJson,
            string path = null,
            long storedBytes = 0)
        {
            RecordId = recordId;
            Sequence = sequence;
            OccurredAtUnixNanoseconds = occurredAtUnixNanoseconds;
            LogJson = logJson;
            Path = path;
            StoredBytes = storedBytes;
        }

        internal string RecordId { get; private set; }
        internal long Sequence { get; private set; }
        internal string OccurredAtUnixNanoseconds { get; private set; }
        internal string LogJson { get; private set; }
        internal string Path { get; private set; }
        internal long StoredBytes { get; private set; }
    }

    internal interface IChillStore
    {
        bool Append(ChillQueuedRecord record);
        IReadOnlyList<ChillQueuedRecord> Peek(int maximumRecords);
        void Acknowledge(IReadOnlyList<ChillQueuedRecord> records);
        void Purge();
    }

    internal sealed class ChillMemoryStore : IChillStore
    {
        private readonly List<ChillQueuedRecord> records = new List<ChillQueuedRecord>();

        public bool Append(ChillQueuedRecord record)
        {
            records.Add(record);
            return true;
        }

        public IReadOnlyList<ChillQueuedRecord> Peek(int maximumRecords)
        {
            int count = Math.Min(maximumRecords, records.Count);
            return records.GetRange(0, count);
        }

        public void Acknowledge(IReadOnlyList<ChillQueuedRecord> acknowledged)
        {
            int count = Math.Min(acknowledged.Count, records.Count);
            records.RemoveRange(0, count);
        }

        public void Purge()
        {
            records.Clear();
        }
    }

    internal sealed class ChillFileStore : IChillStore
    {
        private const string Magic = "CHILLQ1\n";
        private readonly string directory;
        private readonly long maximumBytes;
        private readonly Action<string> diagnostic;
        private long currentBytes;

        internal ChillFileStore(string directory, long maximumBytes, Action<string> diagnostic)
        {
            this.directory = directory;
            this.maximumBytes = maximumBytes;
            this.diagnostic = diagnostic;
            Directory.CreateDirectory(directory);
            Recover();
        }

        public bool Append(ChillQueuedRecord record)
        {
            byte[] bytes = Encoding.UTF8.GetBytes(Magic + record.LogJson);
            if (currentBytes + bytes.LongLength > maximumBytes)
            {
                diagnostic("queue_full");
                return false;
            }

            string prefix = record.OccurredAtUnixNanoseconds.PadLeft(19, '0')
                + "-"
                + record.Sequence.ToString("D16", CultureInfo.InvariantCulture)
                + "-"
                + record.RecordId;
            string finalPath = System.IO.Path.Combine(directory, prefix + ".json");
            string temporaryPath = System.IO.Path.Combine(directory, "." + prefix + ".tmp");
            try
            {
                using (var stream = new FileStream(
                    temporaryPath,
                    FileMode.CreateNew,
                    FileAccess.Write,
                    FileShare.None,
                    4096,
                    FileOptions.None))
                {
                    stream.Write(bytes, 0, bytes.Length);
                    stream.Flush();
                    try
                    {
                        stream.Flush(true);
                    }
                    catch (PlatformNotSupportedException)
                    {
                        // WebGL and some console filesystems expose only the virtual flush.
                    }
                }
                File.Move(temporaryPath, finalPath);
                currentBytes += bytes.LongLength;
                return true;
            }
            catch
            {
                TryDelete(temporaryPath);
                diagnostic("storage_failed");
                return false;
            }
        }

        public IReadOnlyList<ChillQueuedRecord> Peek(int maximumRecords)
        {
            var selected = new List<ChillQueuedRecord>();
            string[] files;
            try
            {
                files = Directory.GetFiles(directory, "*.json", SearchOption.TopDirectoryOnly);
                Array.Sort(files, StringComparer.Ordinal);
            }
            catch
            {
                diagnostic("storage_failed");
                return selected;
            }

            for (int index = 0; index < files.Length && selected.Count < maximumRecords; index += 1)
            {
                try
                {
                    string body = File.ReadAllText(files[index], Encoding.UTF8);
                    if (!body.StartsWith(Magic, StringComparison.Ordinal))
                    {
                        Quarantine(files[index], true);
                        continue;
                    }
                    string name = System.IO.Path.GetFileNameWithoutExtension(files[index]);
                    string[] parts = name.Split(new[] { '-' }, 4);
                    long sequence = 0;
                    if (parts.Length != 4 || !long.TryParse(parts[1], NumberStyles.None, CultureInfo.InvariantCulture, out sequence))
                    {
                        Quarantine(files[index], true);
                        continue;
                    }
                    var info = new FileInfo(files[index]);
                    selected.Add(new ChillQueuedRecord(
                        parts[2] + "-" + parts[3],
                        sequence,
                        parts[0],
                        body.Substring(Magic.Length),
                        files[index],
                        info.Length));
                }
                catch
                {
                    Quarantine(files[index], true);
                }
            }
            return selected;
        }

        public void Acknowledge(IReadOnlyList<ChillQueuedRecord> records)
        {
            for (int index = 0; index < records.Count; index += 1)
            {
                ChillQueuedRecord record = records[index];
                if (string.IsNullOrEmpty(record.Path)
                    || !string.Equals(System.IO.Path.GetDirectoryName(record.Path), directory, StringComparison.Ordinal))
                {
                    continue;
                }
                try
                {
                    File.Delete(record.Path);
                    currentBytes = Math.Max(0, currentBytes - record.StoredBytes);
                }
                catch
                {
                    diagnostic("storage_failed");
                }
            }
        }

        public void Purge()
        {
            try
            {
                string[] files = Directory.GetFiles(directory, "*", SearchOption.TopDirectoryOnly);
                for (int index = 0; index < files.Length; index += 1)
                {
                    TryDelete(files[index]);
                }
                currentBytes = 0;
            }
            catch
            {
                diagnostic("storage_failed");
            }
        }

        private void Recover()
        {
            try
            {
                string[] temporary = Directory.GetFiles(directory, "*.tmp", SearchOption.TopDirectoryOnly);
                for (int index = 0; index < temporary.Length; index += 1)
                {
                    TryDelete(temporary[index]);
                }

                string[] files = Directory.GetFiles(directory, "*.json", SearchOption.TopDirectoryOnly);
                for (int index = 0; index < files.Length; index += 1)
                {
                    try
                    {
                        string body = File.ReadAllText(files[index], Encoding.UTF8);
                        if (!body.StartsWith(Magic, StringComparison.Ordinal))
                        {
                            Quarantine(files[index], false);
                            continue;
                        }
                        currentBytes += new FileInfo(files[index]).Length;
                    }
                    catch
                    {
                        Quarantine(files[index], false);
                    }
                }
            }
            catch
            {
                diagnostic("storage_failed");
            }
        }

        private void Quarantine(string path, bool wasCounted)
        {
            long storedBytes = 0;
            if (wasCounted)
            {
                try
                {
                    storedBytes = new FileInfo(path).Length;
                }
                catch
                {
                    // The record still leaves the readable queue below.
                }
            }
            try
            {
                string target = path + ".corrupt";
                if (File.Exists(target))
                {
                    target += "." + DateTimeOffset.UtcNow.ToUnixTimeMilliseconds().ToString(CultureInfo.InvariantCulture);
                }
                File.Move(path, target);
                diagnostic("queue_corrupt");
            }
            catch
            {
                TryDelete(path);
                diagnostic("storage_failed");
            }
            currentBytes = Math.Max(0, currentBytes - storedBytes);
        }

        private static void TryDelete(string path)
        {
            try
            {
                if (File.Exists(path))
                {
                    File.Delete(path);
                }
            }
            catch
            {
                // The bounded diagnostic at the caller is sufficient.
            }
        }
    }

    internal static class ChillInstallation
    {
        internal static string LoadOrCreate(string queueDirectory, string configured)
        {
            if (!string.IsNullOrEmpty(configured))
            {
                return configured;
            }

            string parent;
            string path;
            try
            {
                parent = Directory.GetParent(queueDirectory) == null
                    ? queueDirectory
                    : Directory.GetParent(queueDirectory).FullName;
                Directory.CreateDirectory(parent);
                path = System.IO.Path.Combine(parent, "installation-id");
                if (File.Exists(path))
                {
                    string existing = File.ReadAllText(path, Encoding.UTF8).Trim();
                    if (ChillIds.IsUuidV4(existing))
                    {
                        return existing;
                    }
                }
            }
            catch
            {
                return ChillIds.NewUuidV4();
            }

            string generated = ChillIds.NewUuidV4();
            string temporary = path + ".tmp";
            try
            {
                File.WriteAllText(temporary, generated + "\n", Encoding.UTF8);
                if (File.Exists(path))
                {
                    File.Delete(path);
                }
                File.Move(temporary, path);
            }
            catch
            {
                try
                {
                    if (File.Exists(temporary))
                    {
                        File.Delete(temporary);
                    }
                }
                catch
                {
                    // The generated identity remains valid for this process.
                }
            }
            return generated;
        }
    }
}
