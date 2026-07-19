import { ChillAnnotation, ChillPage, ChillProvider, useChillAction } from "@chill-observability/browser/react";
import { ChillBrowser, annotationKey } from "@chill-observability/browser";

const accountTier = annotationKey<string>("account.tier");
const client = new ChillBrowser({ endpoint: "https://telemetry.internal", sdkKey: "sdk", policyVersion: "internal-v1", consent: "granted" });

function ComposeButton() {
  const compose = useChillAction("message.compose");
  return <button onClick={compose}>Compose</button>;
}

export function Application() {
  return <ChillProvider client={client}><ChillAnnotation annotation={accountTier} value="internal"><ChillPage name="inbox"><ComposeButton /></ChillPage></ChillAnnotation></ChillProvider>;
}
