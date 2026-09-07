// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#include "ChillTypes.h"

namespace
{
	constexpr int32 MaximumNameLength = 128;
	constexpr int32 MaximumStringLength = 256;
	constexpr int32 MaximumArrayLength = 32;

	bool IsLowerAlpha(TCHAR Character)
	{
		return Character >= TEXT('a') && Character <= TEXT('z');
	}

	bool IsDigit(TCHAR Character)
	{
		return Character >= TEXT('0') && Character <= TEXT('9');
	}

	/** Matches ^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$ without pulling in a regex engine. */
	bool MatchesSemantic(const FString& Value)
	{
		const int32 Length = Value.Len();
		if (Length == 0 || !IsLowerAlpha(Value[0]))
		{
			return false;
		}
		bool bPreviousWasSeparator = false;
		for (int32 Index = 1; Index < Length; ++Index)
		{
			const TCHAR Character = Value[Index];
			const bool bSeparator = Character == TEXT('.') || Character == TEXT('_') || Character == TEXT('-');
			if (bSeparator)
			{
				// A separator may never lead, trail, or repeat.
				if (bPreviousWasSeparator || Index == Length - 1)
				{
					return false;
				}
				bPreviousWasSeparator = true;
				continue;
			}
			if (!IsLowerAlpha(Character) && !IsDigit(Character))
			{
				return false;
			}
			bPreviousWasSeparator = false;
		}
		return true;
	}

	/** Matches ^[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*$. */
	bool MatchesAnnotationKey(const FString& Value)
	{
		const int32 Length = Value.Len();
		if (Length == 0)
		{
			return false;
		}
		bool bSegmentStart = true;
		for (int32 Index = 0; Index < Length; ++Index)
		{
			const TCHAR Character = Value[Index];
			if (Character == TEXT('.'))
			{
				// A dot may never lead, trail, or repeat.
				if (bSegmentStart || Index == Length - 1)
				{
					return false;
				}
				bSegmentStart = true;
				continue;
			}
			if (bSegmentStart)
			{
				if (!IsLowerAlpha(Character))
				{
					return false;
				}
				bSegmentStart = false;
				continue;
			}
			if (!IsLowerAlpha(Character) && !IsDigit(Character) && Character != TEXT('_'))
			{
				return false;
			}
		}
		return true;
	}
}

FChillDiagnostic::FChillDiagnostic(FString InCode, FString InDetail)
	: Code(MoveTemp(InCode))
	, Detail(MoveTemp(InDetail))
	, Timestamp(FDateTime::UtcNow())
{
}

namespace ChillNames
{
	bool IsSemantic(const FString& Value)
	{
		return Value.Len() <= MaximumNameLength && MatchesSemantic(Value);
	}

	bool IsAnnotationKey(const FString& Value)
	{
		return Value.Len() <= MaximumNameLength && MatchesAnnotationKey(Value);
	}

	FString SnakeCase(EChillActionActivation Value)
	{
		switch (Value)
		{
		case EChillActionActivation::Primary: return TEXT("primary");
		case EChillActionActivation::Submit: return TEXT("submit");
		case EChillActionActivation::Toggle: return TEXT("toggle");
		case EChillActionActivation::Selection: return TEXT("selection");
		case EChillActionActivation::Adjust: return TEXT("adjust");
		case EChillActionActivation::Gesture: return TEXT("gesture");
		case EChillActionActivation::System: return TEXT("system");
		}
		return TEXT("primary");
	}

	FString SnakeCase(EChillInput Value)
	{
		switch (Value)
		{
		case EChillInput::Touch: return TEXT("touch");
		case EChillInput::Pointer: return TEXT("pointer");
		case EChillInput::Keyboard: return TEXT("keyboard");
		case EChillInput::Remote: return TEXT("remote");
		case EChillInput::Accessibility: return TEXT("accessibility");
		case EChillInput::Voice: return TEXT("voice");
		case EChillInput::System: return TEXT("system");
		case EChillInput::Unknown: return TEXT("unknown");
		}
		return TEXT("unknown");
	}

	FString SnakeCase(EChillActivityKind Value)
	{
		switch (Value)
		{
		case EChillActivityKind::Ui: return TEXT("ui");
		case EChillActivityKind::Domain: return TEXT("domain");
		case EChillActivityKind::Network: return TEXT("network");
		case EChillActivityKind::Storage: return TEXT("storage");
		case EChillActivityKind::Task: return TEXT("task");
		case EChillActivityKind::Custom: return TEXT("custom");
		}
		return TEXT("domain");
	}

	FString SnakeCase(EChillEventClass Value)
	{
		switch (Value)
		{
		case EChillEventClass::Lifecycle: return TEXT("lifecycle");
		case EChillEventClass::Domain: return TEXT("domain");
		case EChillEventClass::Error: return TEXT("error");
		case EChillEventClass::Crash: return TEXT("crash");
		case EChillEventClass::Performance: return TEXT("performance");
		case EChillEventClass::Experiment: return TEXT("experiment");
		case EChillEventClass::Custom: return TEXT("custom");
		}
		return TEXT("domain");
	}

	FString SnakeCase(EChillEventSeverity Value)
	{
		switch (Value)
		{
		case EChillEventSeverity::Trace: return TEXT("trace");
		case EChillEventSeverity::Debug: return TEXT("debug");
		case EChillEventSeverity::Info: return TEXT("info");
		case EChillEventSeverity::Warn: return TEXT("warn");
		case EChillEventSeverity::Error: return TEXT("error");
		case EChillEventSeverity::Fatal: return TEXT("fatal");
		}
		return TEXT("info");
	}

	FString SnakeCase(EChillPageRelation Value)
	{
		switch (Value)
		{
		case EChillPageRelation::Root: return TEXT("root");
		case EChillPageRelation::Push: return TEXT("push");
		case EChillPageRelation::Tab: return TEXT("tab");
		case EChillPageRelation::Split: return TEXT("split");
		case EChillPageRelation::Sheet: return TEXT("sheet");
		case EChillPageRelation::Popover: return TEXT("popover");
		case EChillPageRelation::Overlay: return TEXT("overlay");
		case EChillPageRelation::Cover: return TEXT("cover");
		}
		return TEXT("root");
	}

	FString SnakeCase(EChillPageCause Value)
	{
		switch (Value)
		{
		case EChillPageCause::Initial: return TEXT("initial");
		case EChillPageCause::Navigate: return TEXT("navigate");
		case EChillPageCause::Back: return TEXT("back");
		case EChillPageCause::Selection: return TEXT("selection");
		case EChillPageCause::Present: return TEXT("present");
		case EChillPageCause::Dismiss: return TEXT("dismiss");
		case EChillPageCause::Replace: return TEXT("replace");
		case EChillPageCause::DeepLink: return TEXT("deep_link");
		case EChillPageCause::Restore: return TEXT("restore");
		case EChillPageCause::Adaptive: return TEXT("adaptive");
		case EChillPageCause::Background: return TEXT("background");
		case EChillPageCause::Foreground: return TEXT("foreground");
		case EChillPageCause::SurfaceDestroyed: return TEXT("surface_destroyed");
		}
		return TEXT("navigate");
	}

	FString SnakeCase(EChillOutcome Value)
	{
		switch (Value)
		{
		case EChillOutcome::Ok: return TEXT("ok");
		case EChillOutcome::Error: return TEXT("error");
		case EChillOutcome::Cancelled: return TEXT("cancelled");
		case EChillOutcome::Timeout: return TEXT("timeout");
		}
		return TEXT("ok");
	}

	FString SnakeCase(EChillAnnotationClassification Value)
	{
		switch (Value)
		{
		case EChillAnnotationClassification::Public: return TEXT("public");
		case EChillAnnotationClassification::Internal: return TEXT("internal");
		case EChillAnnotationClassification::PseudonymousIdentifier: return TEXT("pseudonymous_identifier");
		}
		return TEXT("internal");
	}
}

bool FChillValue::FromString(const FString& Value, FChillValue& OutValue)
{
	if (Value.Len() > MaximumStringLength)
	{
		return false;
	}
	OutValue = FChillValue();
	OutValue.Kind = EChillValueKind::String;
	OutValue.StringValue = Value;
	return true;
}

bool FChillValue::FromBoolean(bool Value, FChillValue& OutValue)
{
	OutValue = FChillValue();
	OutValue.Kind = EChillValueKind::Boolean;
	OutValue.BooleanValue = Value;
	return true;
}

bool FChillValue::FromNumber(double Value, FChillValue& OutValue)
{
	if (FMath::IsNaN(Value) || !FMath::IsFinite(Value))
	{
		return false;
	}
	OutValue = FChillValue();
	OutValue.Kind = EChillValueKind::Number;
	OutValue.NumberValue = Value;
	return true;
}

bool FChillValue::FromStrings(const TArray<FString>& Values, FChillValue& OutValue)
{
	if (Values.Num() > MaximumArrayLength)
	{
		return false;
	}
	for (const FString& Value : Values)
	{
		if (Value.Len() > MaximumStringLength)
		{
			return false;
		}
	}
	OutValue = FChillValue();
	OutValue.Kind = EChillValueKind::Strings;
	OutValue.StringValues = Values;
	return true;
}

bool FChillValue::FromBooleans(const TArray<bool>& Values, FChillValue& OutValue)
{
	if (Values.Num() > MaximumArrayLength)
	{
		return false;
	}
	OutValue = FChillValue();
	OutValue.Kind = EChillValueKind::Booleans;
	OutValue.BooleanValues = Values;
	return true;
}

bool FChillValue::FromNumbers(const TArray<double>& Values, FChillValue& OutValue)
{
	if (Values.Num() > MaximumArrayLength)
	{
		return false;
	}
	for (const double Value : Values)
	{
		if (FMath::IsNaN(Value) || !FMath::IsFinite(Value))
		{
			return false;
		}
	}
	OutValue = FChillValue();
	OutValue.Kind = EChillValueKind::Numbers;
	OutValue.NumberValues = Values;
	return true;
}

FChillAnnotations FChillAnnotations::With(const FString& Key, const FString& Value) const
{
	FChillValue Converted;
	return FChillValue::FromString(Value, Converted) ? Add(Key, Converted) : *this;
}

FChillAnnotations FChillAnnotations::With(const FString& Key, bool Value) const
{
	FChillValue Converted;
	return FChillValue::FromBoolean(Value, Converted) ? Add(Key, Converted) : *this;
}

FChillAnnotations FChillAnnotations::With(const FString& Key, double Value) const
{
	FChillValue Converted;
	return FChillValue::FromNumber(Value, Converted) ? Add(Key, Converted) : *this;
}

FChillAnnotations FChillAnnotations::With(const FString& Key, const TArray<FString>& Values) const
{
	FChillValue Converted;
	return FChillValue::FromStrings(Values, Converted) ? Add(Key, Converted) : *this;
}

FChillAnnotations FChillAnnotations::With(const FString& Key, const TArray<bool>& Values) const
{
	FChillValue Converted;
	return FChillValue::FromBooleans(Values, Converted) ? Add(Key, Converted) : *this;
}

FChillAnnotations FChillAnnotations::With(const FString& Key, const TArray<double>& Values) const
{
	FChillValue Converted;
	return FChillValue::FromNumbers(Values, Converted) ? Add(Key, Converted) : *this;
}

FChillAnnotations FChillAnnotations::MergeDescendant(const FChillAnnotations& Descendant) const
{
	FChillAnnotations Merged = *this;
	for (const FChillAnnotationEntry& Candidate : Descendant.Entries)
	{
		if (!Merged.Contains(Candidate.Key))
		{
			Merged.Entries.Add(Candidate);
		}
	}
	return Merged;
}

FChillAnnotations FChillAnnotations::Add(const FString& Key, const FChillValue& Value) const
{
	if (!ChillNames::IsAnnotationKey(Key) || Contains(Key))
	{
		return *this;
	}
	FChillAnnotations Copied = *this;
	Copied.Entries.Emplace(Key, Value);
	return Copied;
}

bool FChillAnnotations::Contains(const FString& Key) const
{
	for (const FChillAnnotationEntry& Entry : Entries)
	{
		if (Entry.Key.Equals(Key, ESearchCase::CaseSensitive))
		{
			return true;
		}
	}
	return false;
}
