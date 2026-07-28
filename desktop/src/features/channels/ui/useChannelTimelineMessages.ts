import * as React from "react";

import { formatTimelineMessages } from "@/features/messages/lib/formatTimelineMessages";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { Channel, RelayEvent, RespondToMode } from "@/shared/api/types";
import { useAppLocale } from "@/shared/i18n/useAppLocale";

type UseChannelTimelineMessagesOptions = {
  activeChannel: Channel | null;
  channelMembers?: Parameters<typeof formatTimelineMessages>[5];
  currentPubkey?: string;
  currentUserAvatarUrl: string | null;
  messageOwnerProfiles?: UserProfileLookup;
  messageProfiles?: UserProfileLookup;
  personaLookup?: Map<string, string>;
  relaySelfPubkey?: string | null;
  resolvedMessages: RelayEvent[];
  respondToLookup?: Map<string, RespondToMode>;
};

/**
 * Channel timeline rows, formatted in the user's interface language.
 *
 * Extracted from `ChannelScreen` rather than reading the locale there: the
 * screen sits on a ratchet in `scripts/check-file-sizes.mjs`, and the file is
 * meant to shrink. Owning the locale next to the call that needs it also keeps
 * the dependency honest — `locale` belongs in this memo's dependency list, and
 * a missing entry here means the timeline keeps rendering the previous
 * language's dates after a switch.
 */
export function useChannelTimelineMessages({
  activeChannel,
  channelMembers,
  currentPubkey,
  currentUserAvatarUrl,
  messageOwnerProfiles,
  messageProfiles,
  personaLookup,
  relaySelfPubkey,
  resolvedMessages,
  respondToLookup,
}: UseChannelTimelineMessagesOptions): TimelineMessage[] {
  const { locale } = useAppLocale();

  return React.useMemo(
    () =>
      formatTimelineMessages(
        resolvedMessages,
        activeChannel,
        currentPubkey,
        currentUserAvatarUrl,
        messageProfiles,
        channelMembers,
        personaLookup,
        respondToLookup,
        relaySelfPubkey,
        messageOwnerProfiles,
        locale,
      ),
    [
      activeChannel,
      channelMembers,
      currentPubkey,
      currentUserAvatarUrl,
      locale,
      messageOwnerProfiles,
      messageProfiles,
      personaLookup,
      relaySelfPubkey,
      resolvedMessages,
      respondToLookup,
    ],
  );
}
