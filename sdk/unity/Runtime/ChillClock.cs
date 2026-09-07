using System;
using System.Diagnostics;
using System.Globalization;

namespace Chill.Unity
{
    internal interface IChillClock
    {
        long UnixMilliseconds { get; }
        string WallUnixNanoseconds { get; }
        string MonotonicNanoseconds { get; }
    }

    internal sealed class ChillSystemClock : IChillClock
    {
        private const long UnixEpochTicks = 621355968000000000L;

        public long UnixMilliseconds
        {
            get { return DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(); }
        }

        public string WallUnixNanoseconds
        {
            get
            {
                long ticks = DateTime.UtcNow.Ticks - UnixEpochTicks;
                return (ticks * 100L).ToString(CultureInfo.InvariantCulture);
            }
        }

        public string MonotonicNanoseconds
        {
            get
            {
                double nanoseconds = Stopwatch.GetTimestamp() * (1000000000d / Stopwatch.Frequency);
                return ((long)nanoseconds).ToString(CultureInfo.InvariantCulture);
            }
        }
    }
}
