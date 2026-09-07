// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#include "ChillJson.h"

namespace ChillJson
{
	FString Quote(const FString& Value)
	{
		FString Result;
		Result.Reserve(Value.Len() + 8);
		Result.AppendChar(TEXT('"'));
		for (int32 Index = 0; Index < Value.Len(); ++Index)
		{
			const TCHAR Character = Value[Index];
			switch (Character)
			{
			case TEXT('"'):
				Result.Append(TEXT("\\\""));
				break;
			case TEXT('\\'):
				Result.Append(TEXT("\\\\"));
				break;
			case TEXT('\b'):
				Result.Append(TEXT("\\b"));
				break;
			case TEXT('\f'):
				Result.Append(TEXT("\\f"));
				break;
			case TEXT('\n'):
				Result.Append(TEXT("\\n"));
				break;
			case TEXT('\r'):
				Result.Append(TEXT("\\r"));
				break;
			case TEXT('\t'):
				Result.Append(TEXT("\\t"));
				break;
			default:
				if (Character < 0x20)
				{
					Result.Append(FString::Printf(TEXT("\\u%04x"), static_cast<int32>(Character)));
				}
				else
				{
					Result.AppendChar(Character);
				}
				break;
			}
		}
		Result.AppendChar(TEXT('"'));
		return Result;
	}

	FString StringValue(const FString& Value)
	{
		return TEXT("{\"stringValue\":") + Quote(Value) + TEXT("}");
	}

	FString BooleanValue(bool Value)
	{
		return FString(TEXT("{\"boolValue\":")) + (Value ? TEXT("true") : TEXT("false")) + TEXT("}");
	}

	FString IntegerValue(int64 Value)
	{
		return IntegerValue(FString::Printf(TEXT("%lld"), Value));
	}

	FString IntegerValue(const FString& Value)
	{
		return TEXT("{\"intValue\":") + Quote(Value) + TEXT("}");
	}

	FString DoubleValue(double Value)
	{
		// Shortest round-trippable form, matching the "R" format the other SDKs use.
		FString Text = FString::Printf(TEXT("%.17g"), Value);
		for (int32 Precision = 1; Precision < 17; ++Precision)
		{
			const FString Candidate = FString::Printf(TEXT("%.*g"), Precision, Value);
			if (FCString::Atod(*Candidate) == Value)
			{
				Text = Candidate;
				break;
			}
		}
		return TEXT("{\"doubleValue\":") + Text + TEXT("}");
	}

	FString StringArrayValue(const TArray<FString>& Values)
	{
		FString Result = TEXT("{\"arrayValue\":{\"values\":[");
		for (int32 Index = 0; Index < Values.Num(); ++Index)
		{
			if (Index > 0)
			{
				Result.AppendChar(TEXT(','));
			}
			Result.Append(StringValue(Values[Index]));
		}
		Result.Append(TEXT("]}}"));
		return Result;
	}

	FString Attribute(const FString& Key, const FString& AnyValueJson)
	{
		return TEXT("{\"key\":") + Quote(Key) + TEXT(",\"value\":") + AnyValueJson + TEXT("}");
	}

	static FString BooleanArrayValue(const TArray<bool>& Values)
	{
		FString Result = TEXT("{\"arrayValue\":{\"values\":[");
		for (int32 Index = 0; Index < Values.Num(); ++Index)
		{
			if (Index > 0)
			{
				Result.AppendChar(TEXT(','));
			}
			Result.Append(BooleanValue(Values[Index]));
		}
		Result.Append(TEXT("]}}"));
		return Result;
	}

	static FString NumberArrayValue(const TArray<double>& Values)
	{
		FString Result = TEXT("{\"arrayValue\":{\"values\":[");
		for (int32 Index = 0; Index < Values.Num(); ++Index)
		{
			if (Index > 0)
			{
				Result.AppendChar(TEXT(','));
			}
			Result.Append(DoubleValue(Values[Index]));
		}
		Result.Append(TEXT("]}}"));
		return Result;
	}

	FString AnyValue(const FChillValue& Value)
	{
		switch (Value.Kind)
		{
		case EChillValueKind::String:
			return StringValue(Value.StringValue);
		case EChillValueKind::Boolean:
			return BooleanValue(Value.BooleanValue);
		case EChillValueKind::Number:
			return DoubleValue(Value.NumberValue);
		case EChillValueKind::Strings:
			return StringArrayValue(Value.StringValues);
		case EChillValueKind::Booleans:
			return BooleanArrayValue(Value.BooleanValues);
		case EChillValueKind::Numbers:
			return NumberArrayValue(Value.NumberValues);
		}
		return StringValue(FString());
	}

	FString Array(const TArray<FString>& EncodedValues)
	{
		FString Result = TEXT("[");
		for (int32 Index = 0; Index < EncodedValues.Num(); ++Index)
		{
			if (Index > 0)
			{
				Result.AppendChar(TEXT(','));
			}
			Result.Append(EncodedValues[Index]);
		}
		Result.AppendChar(TEXT(']'));
		return Result;
	}
}
