import { getRelayHttpUrl } from "@/shared/api/tauri";
import { getCachedRelayOrigin } from "@/shared/lib/mediaUrl";

/** Resolve the active relay HTTP base used for canonical repository URLs. */
export async function resolveProjectRelayBase(): Promise<string | null> {
  const cached = getCachedRelayOrigin();
  if (cached) return cached;

  try {
    const relayHttpUrl = (await getRelayHttpUrl()).trim();
    return relayHttpUrl || null;
  } catch {
    return null;
  }
}
