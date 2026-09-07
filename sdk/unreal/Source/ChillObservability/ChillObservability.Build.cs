// Copyright Chill Contributors. Licensed under the Apache License, Version 2.0.

using UnrealBuildTool;

public class ChillObservability : ModuleRules
{
	public ChillObservability(ReadOnlyTargetRules Target) : base(Target)
	{
		PCHUsage = PCHUsageMode.UseExplicitOrSharedPCHs;
		bUseUnity = false;
		IWYUSupport = IWYUSupport.Full;

		// The runtime deliberately depends only on first-party engine modules so the
		// plugin ships as source with no third-party or native binary surface.
		PublicDependencyModuleNames.AddRange(new string[]
		{
			"Core",
			"CoreUObject",
			"Engine",
		});

		PrivateDependencyModuleNames.AddRange(new string[]
		{
			"HTTP",
		});
	}
}
