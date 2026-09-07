// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#include "ChillIds.h"

#include "Misc/Guid.h"

namespace
{
	/**
	 * Fills a buffer from the platform GUID source. FGuid::NewGuid draws from the
	 * platform's cryptographic generator on every target the plugin supports, which
	 * keeps identifiers unpredictable without adding a third-party dependency.
	 */
	void RandomBytes(uint8* Bytes, int32 Count)
	{
		int32 Written = 0;
		while (Written < Count)
		{
			const FGuid Guid = FGuid::NewGuid();
			uint32 Words[4] = { Guid.A, Guid.B, Guid.C, Guid.D };
			for (int32 WordIndex = 0; WordIndex < 4 && Written < Count; ++WordIndex)
			{
				for (int32 ByteIndex = 0; ByteIndex < 4 && Written < Count; ++ByteIndex)
				{
					Bytes[Written++] = static_cast<uint8>((Words[WordIndex] >> (ByteIndex * 8)) & 0xff);
				}
			}
		}
	}

	FString Hex(const uint8* Bytes, int32 Count)
	{
		FString Result;
		Result.Reserve(Count * 2);
		for (int32 Index = 0; Index < Count; ++Index)
		{
			Result.Append(FString::Printf(TEXT("%02x"), Bytes[Index]));
		}
		return Result;
	}

	FString FormatUuid(const uint8* Bytes)
	{
		const FString Value = Hex(Bytes, 16);
		return Value.Mid(0, 8) + TEXT("-")
			+ Value.Mid(8, 4) + TEXT("-")
			+ Value.Mid(12, 4) + TEXT("-")
			+ Value.Mid(16, 4) + TEXT("-")
			+ Value.Mid(20, 12);
	}

	FString HexNonZero(int32 Count)
	{
		for (;;)
		{
			TArray<uint8> Bytes;
			Bytes.SetNumUninitialized(Count);
			RandomBytes(Bytes.GetData(), Count);
			for (int32 Index = 0; Index < Count; ++Index)
			{
				if (Bytes[Index] != 0)
				{
					return Hex(Bytes.GetData(), Count);
				}
			}
		}
	}

	bool IsHexDigit(TCHAR Character)
	{
		return (Character >= TEXT('0') && Character <= TEXT('9'))
			|| (Character >= TEXT('a') && Character <= TEXT('f'));
	}
}

namespace ChillIds
{
	bool IsUuidV4(const FString& Value)
	{
		// ^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$
		if (Value.Len() != 36)
		{
			return false;
		}
		static const int32 DashPositions[] = { 8, 13, 18, 23 };
		for (const int32 Position : DashPositions)
		{
			if (Value[Position] != TEXT('-'))
			{
				return false;
			}
		}
		for (int32 Index = 0; Index < 36; ++Index)
		{
			if (Index == 8 || Index == 13 || Index == 18 || Index == 23)
			{
				continue;
			}
			if (!IsHexDigit(Value[Index]))
			{
				return false;
			}
		}
		if (Value[14] != TEXT('4'))
		{
			return false;
		}
		const TCHAR Variant = Value[19];
		return Variant == TEXT('8') || Variant == TEXT('9') || Variant == TEXT('a') || Variant == TEXT('b');
	}

	FString NewUuidV4()
	{
		uint8 Bytes[16];
		RandomBytes(Bytes, 16);
		Bytes[6] = static_cast<uint8>((Bytes[6] & 0x0f) | 0x40);
		Bytes[8] = static_cast<uint8>((Bytes[8] & 0x3f) | 0x80);
		return FormatUuid(Bytes);
	}

	FString NewUuidV7(int64 UnixMilliseconds)
	{
		uint8 Bytes[16];
		RandomBytes(Bytes, 16);
		uint64 Timestamp = static_cast<uint64>(UnixMilliseconds);
		for (int32 Index = 5; Index >= 0; --Index)
		{
			Bytes[Index] = static_cast<uint8>(Timestamp & 0xff);
			Timestamp >>= 8;
		}
		Bytes[6] = static_cast<uint8>((Bytes[6] & 0x0f) | 0x70);
		Bytes[8] = static_cast<uint8>((Bytes[8] & 0x3f) | 0x80);
		return FormatUuid(Bytes);
	}

	FString NewTraceparent()
	{
		return TEXT("00-") + HexNonZero(16) + TEXT("-") + HexNonZero(8) + TEXT("-01");
	}

	double RandomUnit()
	{
		uint8 Bytes[4];
		RandomBytes(Bytes, 4);
		const uint32 Value = (static_cast<uint32>(Bytes[0]) << 24)
			| (static_cast<uint32>(Bytes[1]) << 16)
			| (static_cast<uint32>(Bytes[2]) << 8)
			| static_cast<uint32>(Bytes[3]);
		return static_cast<double>(Value) / (static_cast<double>(MAX_uint32) + 1.0);
	}

	FString TraceId(const FString& Traceparent)
	{
		return Traceparent.Mid(3, 32);
	}

	FString SpanId(const FString& Traceparent)
	{
		return Traceparent.Mid(36, 16);
	}
}
