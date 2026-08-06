import type { BakedEnvEntry } from "@/shared/api/tauri";

/**
 * Whether a build ships a complete agent configuration, so the harness and
 * model onboarding steps have nothing left to ask.
 *
 * Complete means all three of provider, model, and the provider's required
 * credentials are baked in (see `scripts/schoolx-keyed-build.sh`). Anything
 * less and the steps must still be shown: a build with a provider but no key
 * would otherwise drop the user into an app whose agents fail on first use,
 * with no hint that Settings → Agents is where the missing piece goes.
 *
 * Presence is what counts, not the value — the backend masks secret entries
 * before they cross the IPC boundary, so a baked API key arrives as bullets.
 *
 * `requiredCredentialKeys` is injected rather than imported so this stays a
 * dependency-free module the node test runner can load directly; the caller
 * passes `requiredCredentialEnvKeys` from the agents feature, which is the
 * single source of truth for the provider → credential mapping.
 */
export function bakedAgentConfigIsComplete(
  bakedEnv: readonly BakedEnvEntry[] | undefined | null,
  requiredCredentialKeys: (provider: string) => readonly string[],
): boolean {
  if (!bakedEnv || bakedEnv.length === 0) return false;

  const present = new Set(bakedEnv.map((entry) => entry.key));

  const provider = bakedEnv
    .find((entry) => entry.key === "BUZZ_AGENT_PROVIDER")
    ?.value.trim();
  if (!provider) return false;

  if (!present.has("BUZZ_AGENT_MODEL")) return false;

  return requiredCredentialKeys(provider).every((key) => present.has(key));
}
