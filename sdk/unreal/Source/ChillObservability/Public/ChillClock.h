// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#pragma once

#include "CoreMinimal.h"

/**
 * Wall and monotonic time. Injectable so contract tests can pin timestamps without
 * waiting on real time.
 */
class CHILLOBSERVABILITY_API IChillClock
{
public:
	virtual ~IChillClock() = default;
	virtual int64 UnixMilliseconds() const = 0;
	virtual FString WallUnixNanoseconds() const = 0;
	virtual FString MonotonicNanoseconds() const = 0;
};

class CHILLOBSERVABILITY_API FChillSystemClock final : public IChillClock
{
public:
	virtual int64 UnixMilliseconds() const override;
	virtual FString WallUnixNanoseconds() const override;
	virtual FString MonotonicNanoseconds() const override;
};
