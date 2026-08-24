import * as React from "react";

import { useKnownAgentPubkeys } from "@/features/agents/useKnownAgentPubkeys";
import { useOwnedAgentPubkeys } from "@/features/home/useOwnedAgentPubkeys";
import { collectMessageMentionPubkeys } from "@/features/messages/lib/formatTimelineMessages";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { FeedItem, RelayEvent } from "@/shared/api/types";
import { KIND_REACTION } from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function useHomeInboxProfiles({
  channelMessages,
  currentPubkey,
  feedItems,
  threadEvents,
}: {
  channelMessages: RelayEvent[] | undefined;
  currentPubkey: string | undefined;
  feedItems: FeedItem[];
  threadEvents: RelayEvent[];
}) {
  const feedProfilePubkeys = React.useMemo(
    () => [
      ...new Set([
        ...feedItems.map((item) => item.pubkey),
        ...collectMessageMentionPubkeys(feedItems),
        ...threadEvents.map((event) => event.pubkey),
        ...collectMessageMentionPubkeys(threadEvents),
        ...(channelMessages ?? [])
          .filter((event) => event.kind === KIND_REACTION)
          .map((event) => event.pubkey),
        ...(currentPubkey ? [currentPubkey] : []),
      ]),
    ],
    [channelMessages, currentPubkey, feedItems, threadEvents],
  );
  const feedProfilesQuery = useUsersBatchQuery(feedProfilePubkeys, {
    enabled: feedProfilePubkeys.length > 0,
  });
  const feedProfiles = feedProfilesQuery.data?.profiles;
  const ownedAgentPubkeys = useOwnedAgentPubkeys(
    true,
    feedProfiles,
    currentPubkey,
  );
  const feedOwnerPubkeys = React.useMemo(
    () => [
      ...new Set(
        Object.values(feedProfiles ?? {})
          .map((profile) => profile.ownerPubkey)
          .filter((pubkey): pubkey is string => Boolean(pubkey)),
      ),
    ],
    [feedProfiles],
  );
  const feedOwnerProfilesQuery = useUsersBatchQuery(feedOwnerPubkeys, {
    enabled: feedOwnerPubkeys.length > 0,
  });
  const feedOwnerProfiles = feedOwnerProfilesQuery.data?.profiles;
  const communityAgentPubkeys = useKnownAgentPubkeys();
  const inboxAgentPubkeys = React.useMemo(() => {
    const pubkeys = new Set(communityAgentPubkeys);

    for (const [pubkey, profile] of Object.entries(feedProfiles ?? {})) {
      if (profile.isAgent) {
        pubkeys.add(normalizePubkey(pubkey));
      }
    }

    return pubkeys;
  }, [feedProfiles, communityAgentPubkeys]);

  return {
    feedOwnerProfiles,
    feedProfiles,
    inboxAgentPubkeys,
    ownedAgentPubkeys,
  };
}
