using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Text.RegularExpressions;

namespace Chill.Unity
{
    public enum ChillConsent
    {
        Denied,
        Granted
    }

    public enum ChillAnnotationClassification
    {
        Public,
        Internal,
        PseudonymousIdentifier
    }

    public enum ChillActionActivation
    {
        Primary,
        Submit,
        Toggle,
        Selection,
        Adjust,
        Gesture,
        System
    }

    public enum ChillInput
    {
        Touch,
        Pointer,
        Keyboard,
        Remote,
        Accessibility,
        Voice,
        System,
        Unknown
    }

    public enum ChillActivityKind
    {
        Ui,
        Domain,
        Network,
        Storage,
        Task,
        Custom
    }

    public enum ChillEventClass
    {
        Lifecycle,
        Domain,
        Error,
        Crash,
        Performance,
        Experiment,
        Custom
    }

    public enum ChillEventSeverity
    {
        Trace,
        Debug,
        Info,
        Warn,
        Error,
        Fatal
    }

    public enum ChillPageRelation
    {
        Root,
        Push,
        Tab,
        Split,
        Sheet,
        Popover,
        Overlay,
        Cover
    }

    public enum ChillPageCause
    {
        Initial,
        Navigate,
        Back,
        Selection,
        Present,
        Dismiss,
        Replace,
        DeepLink,
        Restore,
        Adaptive,
        Background,
        Foreground,
        SurfaceDestroyed
    }

    public enum ChillOutcome
    {
        Ok,
        Error,
        Cancelled,
        Timeout
    }

    public sealed class ChillDiagnostic
    {
        internal ChillDiagnostic(string code, string detail)
        {
            Code = code;
            Detail = detail;
            Timestamp = DateTimeOffset.UtcNow;
        }

        public string Code { get; private set; }
        public string Detail { get; private set; }
        public DateTimeOffset Timestamp { get; private set; }
    }

    internal static class ChillNames
    {
        private static readonly Regex SemanticName = new Regex(
            "^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$",
            RegexOptions.CultureInvariant);

        private static readonly Regex AnnotationKey = new Regex(
            "^[a-z][a-z0-9_]*(?:\\.[a-z][a-z0-9_]*)*$",
            RegexOptions.CultureInvariant);

        internal static bool IsSemantic(string value)
        {
            return value != null && value.Length <= 128 && SemanticName.IsMatch(value);
        }

        internal static bool IsAnnotationKey(string value)
        {
            return value != null && value.Length <= 128 && AnnotationKey.IsMatch(value);
        }

        internal static string SnakeCase(Enum value)
        {
            string source = value.ToString();
            var result = new System.Text.StringBuilder(source.Length + 4);
            for (int index = 0; index < source.Length; index += 1)
            {
                char current = source[index];
                if (char.IsUpper(current) && index > 0)
                {
                    result.Append('_');
                }
                result.Append(char.ToLowerInvariant(current));
            }
            return result.ToString();
        }
    }

    internal enum ChillValueKind
    {
        String,
        Boolean,
        Number,
        Strings,
        Booleans,
        Numbers
    }

    internal sealed class ChillValue
    {
        private ChillValue(ChillValueKind kind, object value)
        {
            Kind = kind;
            Value = value;
        }

        internal ChillValueKind Kind { get; private set; }
        internal object Value { get; private set; }

        internal static ChillValue From(string value)
        {
            if (value == null || value.Length > 256)
            {
                throw new ArgumentException("String annotations must contain at most 256 characters.");
            }
            return new ChillValue(ChillValueKind.String, value);
        }

        internal static ChillValue From(bool value)
        {
            return new ChillValue(ChillValueKind.Boolean, value);
        }

        internal static ChillValue From(double value)
        {
            if (double.IsNaN(value) || double.IsInfinity(value))
            {
                throw new ArgumentException("Number annotations must be finite.");
            }
            return new ChillValue(ChillValueKind.Number, value);
        }

        internal static ChillValue From(IReadOnlyList<string> values)
        {
            if (values == null || values.Count > 32)
            {
                throw new ArgumentException("Annotation arrays must contain at most 32 values.");
            }
            var copied = new string[values.Count];
            for (int index = 0; index < values.Count; index += 1)
            {
                if (values[index] == null || values[index].Length > 256)
                {
                    throw new ArgumentException("String annotations must contain at most 256 characters.");
                }
                copied[index] = values[index];
            }
            return new ChillValue(ChillValueKind.Strings, copied);
        }

        internal static ChillValue From(IReadOnlyList<bool> values)
        {
            if (values == null || values.Count > 32)
            {
                throw new ArgumentException("Annotation arrays must contain at most 32 values.");
            }
            var copied = new bool[values.Count];
            for (int index = 0; index < values.Count; index += 1)
            {
                copied[index] = values[index];
            }
            return new ChillValue(ChillValueKind.Booleans, copied);
        }

        internal static ChillValue From(IReadOnlyList<double> values)
        {
            if (values == null || values.Count > 32)
            {
                throw new ArgumentException("Annotation arrays must contain at most 32 values.");
            }
            var copied = new double[values.Count];
            for (int index = 0; index < values.Count; index += 1)
            {
                if (double.IsNaN(values[index]) || double.IsInfinity(values[index]))
                {
                    throw new ArgumentException("Number annotations must be finite.");
                }
                copied[index] = values[index];
            }
            return new ChillValue(ChillValueKind.Numbers, copied);
        }
    }

    internal sealed class ChillAnnotationEntry
    {
        internal ChillAnnotationEntry(string key, ChillValue value)
        {
            Key = key;
            Value = value;
        }

        internal string Key { get; private set; }
        internal ChillValue Value { get; private set; }
    }

    public sealed class ChillAnnotations
    {
        private readonly List<ChillAnnotationEntry> entries;

        public ChillAnnotations()
        {
            entries = new List<ChillAnnotationEntry>();
        }

        private ChillAnnotations(List<ChillAnnotationEntry> entries)
        {
            this.entries = entries;
        }

        internal IReadOnlyList<ChillAnnotationEntry> Entries
        {
            get { return new ReadOnlyCollection<ChillAnnotationEntry>(entries); }
        }

        public ChillAnnotations With(string key, string value)
        {
            return Add(key, ChillValue.From(value));
        }

        public ChillAnnotations With(string key, bool value)
        {
            return Add(key, ChillValue.From(value));
        }

        public ChillAnnotations With(string key, double value)
        {
            return Add(key, ChillValue.From(value));
        }

        public ChillAnnotations With(string key, IReadOnlyList<string> values)
        {
            return Add(key, ChillValue.From(values));
        }

        public ChillAnnotations With(string key, IReadOnlyList<bool> values)
        {
            return Add(key, ChillValue.From(values));
        }

        public ChillAnnotations With(string key, IReadOnlyList<double> values)
        {
            return Add(key, ChillValue.From(values));
        }

        public ChillAnnotations MergeDescendant(ChillAnnotations descendant)
        {
            if (descendant == null)
            {
                return this;
            }
            var merged = new List<ChillAnnotationEntry>(entries);
            for (int index = 0; index < descendant.entries.Count; index += 1)
            {
                ChillAnnotationEntry candidate = descendant.entries[index];
                if (!Contains(merged, candidate.Key))
                {
                    merged.Add(candidate);
                }
            }
            return new ChillAnnotations(merged);
        }

        private ChillAnnotations Add(string key, ChillValue value)
        {
            if (!ChillNames.IsAnnotationKey(key))
            {
                throw new ArgumentException("Annotation keys must be lowercase dotted identifiers.", "key");
            }
            if (Contains(entries, key))
            {
                return this;
            }
            var copied = new List<ChillAnnotationEntry>(entries);
            copied.Add(new ChillAnnotationEntry(key, value));
            return new ChillAnnotations(copied);
        }

        private static bool Contains(List<ChillAnnotationEntry> values, string key)
        {
            for (int index = 0; index < values.Count; index += 1)
            {
                if (string.Equals(values[index].Key, key, StringComparison.Ordinal))
                {
                    return true;
                }
            }
            return false;
        }
    }
}
