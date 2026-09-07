using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;

namespace Chill.Unity
{
    internal static class ChillJson
    {
        internal static string Quote(string value)
        {
            if (value == null)
            {
                return "null";
            }

            var result = new StringBuilder(value.Length + 8);
            result.Append('"');
            for (int index = 0; index < value.Length; index += 1)
            {
                char current = value[index];
                switch (current)
                {
                    case '"':
                        result.Append("\\\"");
                        break;
                    case '\\':
                        result.Append("\\\\");
                        break;
                    case '\b':
                        result.Append("\\b");
                        break;
                    case '\f':
                        result.Append("\\f");
                        break;
                    case '\n':
                        result.Append("\\n");
                        break;
                    case '\r':
                        result.Append("\\r");
                        break;
                    case '\t':
                        result.Append("\\t");
                        break;
                    default:
                        if (current < 0x20)
                        {
                            result.Append("\\u");
                            result.Append(((int)current).ToString("x4", CultureInfo.InvariantCulture));
                        }
                        else
                        {
                            result.Append(current);
                        }
                        break;
                }
            }
            result.Append('"');
            return result.ToString();
        }

        internal static string StringValue(string value)
        {
            return "{\"stringValue\":" + Quote(value) + "}";
        }

        internal static string BooleanValue(bool value)
        {
            return "{\"boolValue\":" + (value ? "true" : "false") + "}";
        }

        internal static string IntegerValue(long value)
        {
            return "{\"intValue\":" + Quote(value.ToString(CultureInfo.InvariantCulture)) + "}";
        }

        internal static string IntegerValue(string value)
        {
            return "{\"intValue\":" + Quote(value) + "}";
        }

        internal static string DoubleValue(double value)
        {
            return "{\"doubleValue\":" + value.ToString("R", CultureInfo.InvariantCulture) + "}";
        }

        internal static string StringArrayValue(IReadOnlyList<string> values)
        {
            var result = new StringBuilder("{\"arrayValue\":{\"values\":[");
            for (int index = 0; index < values.Count; index += 1)
            {
                if (index > 0)
                {
                    result.Append(',');
                }
                result.Append(StringValue(values[index]));
            }
            result.Append("]}}");
            return result.ToString();
        }

        internal static string Attribute(string key, string anyValueJson)
        {
            return "{\"key\":" + Quote(key) + ",\"value\":" + anyValueJson + "}";
        }

        internal static string AnyValue(ChillValue value)
        {
            switch (value.Kind)
            {
                case ChillValueKind.String:
                    return StringValue((string)value.Value);
                case ChillValueKind.Boolean:
                    return BooleanValue((bool)value.Value);
                case ChillValueKind.Number:
                    return DoubleValue((double)value.Value);
                case ChillValueKind.Strings:
                    return StringArrayValue((string[])value.Value);
                case ChillValueKind.Booleans:
                    return BooleanArrayValue((bool[])value.Value);
                case ChillValueKind.Numbers:
                    return NumberArrayValue((double[])value.Value);
                default:
                    throw new InvalidOperationException("Unsupported Chill annotation value.");
            }
        }

        internal static string Array(IEnumerable<string> encodedValues)
        {
            var result = new StringBuilder("[");
            bool first = true;
            foreach (string encoded in encodedValues)
            {
                if (!first)
                {
                    result.Append(',');
                }
                first = false;
                result.Append(encoded);
            }
            result.Append(']');
            return result.ToString();
        }

        private static string BooleanArrayValue(IReadOnlyList<bool> values)
        {
            var result = new StringBuilder("{\"arrayValue\":{\"values\":[");
            for (int index = 0; index < values.Count; index += 1)
            {
                if (index > 0)
                {
                    result.Append(',');
                }
                result.Append(BooleanValue(values[index]));
            }
            result.Append("]}}");
            return result.ToString();
        }

        private static string NumberArrayValue(IReadOnlyList<double> values)
        {
            var result = new StringBuilder("{\"arrayValue\":{\"values\":[");
            for (int index = 0; index < values.Count; index += 1)
            {
                if (index > 0)
                {
                    result.Append(',');
                }
                result.Append(DoubleValue(values[index]));
            }
            result.Append("]}}");
            return result.ToString();
        }
    }
}
