using System;
using Chill.Unity;

namespace Chill.Unity.Samples
{
    public static class BasicChillBootstrap
    {
        public static void Configure(
            string endpoint,
            string sdkKeyFromAppConfiguration,
            bool analyticsConsent)
        {
            var configuration = new ChillConfiguration(
                "sample.game",
                new Uri(endpoint),
                sdkKeyFromAppConfiguration)
            {
                Consent = analyticsConsent
                    ? ChillConsent.Granted
                    : ChillConsent.Denied
            };
            configuration
                .AllowAnnotation("player.cohort", ChillAnnotationClassification.PseudonymousIdentifier)
                .AllowAnnotation("match.mode");
            ChillRuntime.Configure(configuration);
        }
    }
}
