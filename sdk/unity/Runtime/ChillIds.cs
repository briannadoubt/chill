using System;
using System.Globalization;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;

namespace Chill.Unity
{
    internal static class ChillIds
    {
        private static readonly Regex UuidV4 = new Regex(
            "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            RegexOptions.CultureInvariant);

        private static readonly object RandomLock = new object();
        private static readonly RandomNumberGenerator Random = RandomNumberGenerator.Create();

        internal static bool IsUuidV4(string value)
        {
            return value != null && UuidV4.IsMatch(value);
        }

        internal static string NewUuidV4()
        {
            byte[] bytes = RandomBytes(16);
            bytes[6] = (byte)((bytes[6] & 0x0f) | 0x40);
            bytes[8] = (byte)((bytes[8] & 0x3f) | 0x80);
            return FormatUuid(bytes);
        }

        internal static string NewUuidV7(long unixMilliseconds)
        {
            byte[] bytes = RandomBytes(16);
            ulong timestamp = (ulong)unixMilliseconds;
            for (int index = 5; index >= 0; index -= 1)
            {
                bytes[index] = (byte)(timestamp & 0xff);
                timestamp >>= 8;
            }
            bytes[6] = (byte)((bytes[6] & 0x0f) | 0x70);
            bytes[8] = (byte)((bytes[8] & 0x3f) | 0x80);
            return FormatUuid(bytes);
        }

        internal static string NewTraceparent()
        {
            return "00-" + HexNonZero(16) + "-" + HexNonZero(8) + "-01";
        }

        internal static double RandomUnit()
        {
            byte[] bytes = RandomBytes(4);
            uint value = ((uint)bytes[0] << 24)
                | ((uint)bytes[1] << 16)
                | ((uint)bytes[2] << 8)
                | bytes[3];
            return value / ((double)uint.MaxValue + 1d);
        }

        internal static string TraceId(string traceparent)
        {
            return traceparent.Substring(3, 32);
        }

        internal static string SpanId(string traceparent)
        {
            return traceparent.Substring(36, 16);
        }

        private static byte[] RandomBytes(int count)
        {
            var bytes = new byte[count];
            lock (RandomLock)
            {
                Random.GetBytes(bytes);
            }
            return bytes;
        }

        private static string HexNonZero(int count)
        {
            while (true)
            {
                byte[] bytes = RandomBytes(count);
                bool nonZero = false;
                for (int index = 0; index < bytes.Length; index += 1)
                {
                    nonZero |= bytes[index] != 0;
                }
                if (nonZero)
                {
                    return Hex(bytes);
                }
            }
        }

        private static string FormatUuid(byte[] bytes)
        {
            string value = Hex(bytes);
            return value.Substring(0, 8) + "-"
                + value.Substring(8, 4) + "-"
                + value.Substring(12, 4) + "-"
                + value.Substring(16, 4) + "-"
                + value.Substring(20, 12);
        }

        private static string Hex(byte[] bytes)
        {
            var result = new StringBuilder(bytes.Length * 2);
            for (int index = 0; index < bytes.Length; index += 1)
            {
                result.Append(bytes[index].ToString("x2", CultureInfo.InvariantCulture));
            }
            return result.ToString();
        }
    }
}
