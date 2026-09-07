// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

using UnrealBuildTool;

public class ChillObservabilityTests : ModuleRules
{
	public ChillObservabilityTests(ReadOnlyTargetRules Target) : base(Target)
	{
		PCHUsage = PCHUsageMode.UseExplicitOrSharedPCHs;
		bUseUnity = false;

		PrivateDependencyModuleNames.AddRange(new string[]
		{
			"Core",
			"CoreUObject",
			"Engine",
			"ChillObservability",
		});
	}
}
