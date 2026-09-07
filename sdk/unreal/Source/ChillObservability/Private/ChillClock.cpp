// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#include "ChillClock.h"

#include "HAL/PlatformTime.h"

int64 FChillSystemClock::UnixMilliseconds() const
{
	return FDateTime::UtcNow().ToUnixTimestamp() * 1000
		+ static_cast<int64>(FDateTime::UtcNow().GetMillisecond());
}

FString FChillSystemClock::WallUnixNanoseconds() const
{
	const FDateTime Now = FDateTime::UtcNow();
	// Ticks are 100ns since year 1; rebase onto the Unix epoch before scaling.
	const int64 UnixTicks = Now.GetTicks() - FDateTime(1970, 1, 1).GetTicks();
	return FString::Printf(TEXT("%lld"), UnixTicks * 100);
}

FString FChillSystemClock::MonotonicNanoseconds() const
{
	const double Seconds = FPlatformTime::Seconds();
	return FString::Printf(TEXT("%lld"), static_cast<int64>(Seconds * 1000000000.0));
}
