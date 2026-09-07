// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

#pragma once

#include "CoreMinimal.h"

/** Whether the player has granted capture consent. Capture is denied until granted. */
enum class EChillConsent : uint8
{
	Denied,
	Granted
};

/** Privacy classification declared for an annotation key. */
enum class EChillAnnotationClassification : uint8
{
	Public,
	Internal,
	PseudonymousIdentifier
};

enum class EChillActionActivation : uint8
{
	Primary,
	Submit,
	Toggle,
	Selection,
	Adjust,
	Gesture,
	System
};

enum class EChillInput : uint8
{
	Touch,
	Pointer,
	Keyboard,
	Remote,
	Accessibility,
	Voice,
	System,
	Unknown
};

enum class EChillActivityKind : uint8
{
	Ui,
	Domain,
	Network,
	Storage,
	Task,
	Custom
};

enum class EChillEventClass : uint8
{
	Lifecycle,
	Domain,
	Error,
	Crash,
	Performance,
	Experiment,
	Custom
};

enum class EChillEventSeverity : uint8
{
	Trace,
	Debug,
	Info,
	Warn,
	Error,
	Fatal
};

enum class EChillPageRelation : uint8
{
	Root,
	Push,
	Tab,
	Split,
	Sheet,
	Popover,
	Overlay,
	Cover
};

enum class EChillPageCause : uint8
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
};

enum class EChillOutcome : uint8
{
	Ok,
	Error,
	Cancelled,
	Timeout
};

/** A bounded, non-fatal report about dropped or rejected capture. */
struct CHILLOBSERVABILITY_API FChillDiagnostic
{
	FChillDiagnostic() = default;
	FChillDiagnostic(FString InCode, FString InDetail);

	FString Code;
	FString Detail;
	FDateTime Timestamp;
};

/** Name and enum spelling rules shared by every emitted identifier. */
namespace ChillNames
{
	/** Lowercase dotted, dashed or underscored semantic identifier, at most 128 characters. */
	CHILLOBSERVABILITY_API bool IsSemantic(const FString& Value);

	/** Lowercase dotted annotation key, at most 128 characters. */
	CHILLOBSERVABILITY_API bool IsAnnotationKey(const FString& Value);

	CHILLOBSERVABILITY_API FString SnakeCase(EChillActionActivation Value);
	CHILLOBSERVABILITY_API FString SnakeCase(EChillInput Value);
	CHILLOBSERVABILITY_API FString SnakeCase(EChillActivityKind Value);
	CHILLOBSERVABILITY_API FString SnakeCase(EChillEventClass Value);
	CHILLOBSERVABILITY_API FString SnakeCase(EChillEventSeverity Value);
	CHILLOBSERVABILITY_API FString SnakeCase(EChillPageRelation Value);
	CHILLOBSERVABILITY_API FString SnakeCase(EChillPageCause Value);
	CHILLOBSERVABILITY_API FString SnakeCase(EChillOutcome Value);
	CHILLOBSERVABILITY_API FString SnakeCase(EChillAnnotationClassification Value);
}

enum class EChillValueKind : uint8
{
	String,
	Boolean,
	Number,
	Strings,
	Booleans,
	Numbers
};

/**
 * A bounded annotation value. Construction is fallible so an out-of-bounds value is
 * rejected at its declaration source rather than being silently truncated on the wire.
 */
struct CHILLOBSERVABILITY_API FChillValue
{
	EChillValueKind Kind = EChillValueKind::String;
	FString StringValue;
	bool BooleanValue = false;
	double NumberValue = 0.0;
	TArray<FString> StringValues;
	TArray<bool> BooleanValues;
	TArray<double> NumberValues;

	static bool FromString(const FString& Value, FChillValue& OutValue);
	static bool FromBoolean(bool Value, FChillValue& OutValue);
	static bool FromNumber(double Value, FChillValue& OutValue);
	static bool FromStrings(const TArray<FString>& Values, FChillValue& OutValue);
	static bool FromBooleans(const TArray<bool>& Values, FChillValue& OutValue);
	static bool FromNumbers(const TArray<double>& Values, FChillValue& OutValue);
};

struct CHILLOBSERVABILITY_API FChillAnnotationEntry
{
	FChillAnnotationEntry() = default;
	FChillAnnotationEntry(FString InKey, FChillValue InValue)
		: Key(MoveTemp(InKey))
		, Value(MoveTemp(InValue))
	{
	}

	FString Key;
	FChillValue Value;
};

/**
 * An immutable, insertion-ordered annotation set. First write wins, matching the
 * other SDKs, so a descendant scope can never overwrite an ancestor's declaration.
 */
class CHILLOBSERVABILITY_API FChillAnnotations
{
public:
	FChillAnnotations() = default;

	FChillAnnotations With(const FString& Key, const FString& Value) const;
	FChillAnnotations With(const FString& Key, bool Value) const;
	FChillAnnotations With(const FString& Key, double Value) const;
	FChillAnnotations With(const FString& Key, const TArray<FString>& Values) const;
	FChillAnnotations With(const FString& Key, const TArray<bool>& Values) const;
	FChillAnnotations With(const FString& Key, const TArray<double>& Values) const;

	/** Merges a descendant set, keeping this set's value whenever a key collides. */
	FChillAnnotations MergeDescendant(const FChillAnnotations& Descendant) const;

	const TArray<FChillAnnotationEntry>& GetEntries() const
	{
		return Entries;
	}

private:
	FChillAnnotations Add(const FString& Key, const FChillValue& Value) const;
	bool Contains(const FString& Key) const;

	TArray<FChillAnnotationEntry> Entries;
};
