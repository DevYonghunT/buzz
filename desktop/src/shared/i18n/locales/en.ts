export const en = {
  app: {
    /**
     * Product name as shown *inside* the UI. Deliberately separate from the
     * OS-level bundle name in `tauri.conf.json`, which stays ASCII `SchoolX`
     * so app paths, the DMG volume, and Windows/Linux packaging stay
     * predictable. See `shared/product`.
     */
    productName: "SchoolX",
    loading: {
      settingUpCommunity: "Setting up your community…",
      switchingCommunity: "Switching community…",
    },
  },
  settings: {
    sidebar: {
      groups: {
        personal: "Personal",
        communities: "Communities",
        app: "App",
      },
      groupAriaLabel: "{{group}} settings sections",
      backToApp: "Back to app",
      checkingCommunityAccess: "Checking invite permissions…",
      communityAccessCheckFailed: "Invite settings could not be checked.",
      communityAccessUnavailable:
        "Invite settings are unavailable. Relay recovery may still be in progress.",
      tryAgain: "Try again",
    },
    sections: {
      appearance: "Appearance",
      profile: "Profile",
      notifications: "Notifications",
      experimental: "Experiments",
      agents: "Agents",
      channelTemplates: "Templates",
      workspaceCatalog: "SchoolX workspace",
      compute: "Compute",
      shortcuts: "Shortcuts",
      hostedCommunities: "Hosted communities",
      communityMembers: "Invites",
      moderation: "Moderation",
      customEmoji: "Custom emoji",
      localArchive: "Local archive",
      mobile: "Mobile",
      updates: "Updates",
    },
  },
  time: {
    /**
     * Day-divider labels and relative reply times in the message timeline.
     *
     * English keeps the exact wording it shipped with — ordinal dates
     * ("May 19th") and "on <date>" — because adding Korean must not restyle
     * English product copy. Korean has no ordinal suffix and no leading
     * preposition, so `onDate` is the bare date there.
     */
    today: "Today",
    yesterday: "Yesterday",
    justNow: "just now",
    onDate: "on {{date}}",
  },
  appearance: {
    title: "Appearance",
    description: "Choose a theme and interface language.",
    mode: {
      system: "System",
      light: "Light",
      dark: "Dark",
    },
    accentColor: "Accent color",
    threadLayout: {
      title: "Thread layout",
      focus: {
        label: "Focus",
        description: "Threads open over the channel, full width",
      },
      split: {
        label: "Split",
        description: "Threads open in a side panel next to the channel",
      },
    },
    language: {
      title: "Interface language",
      description: "Changes menus and controls on this device.",
      ko: "Korean",
      en: "English",
    },
  },
  catalog: {
    title: "SchoolX default workspace",
    description:
      "Create the standard SchoolX rooms. Nothing is created until you apply.",
    apply: "Apply selected",
    applying: "Applying…",
    openWarningScope:
      "Every signed-in user can read and write without being a member.",
    openWarningAgents:
      "Managed agents still need to be added explicitly before they can join.",
    decision: {
      create_or_recreate: "Will be created",
      resume: "Will resume",
      no_change: "Already applied",
      conflict: "Needs your decision",
      retired: "No longer offered",
      deleted: "Previously deleted",
      adopted: "Existing room reused",
      not_owned: "Owned by someone else",
    },
    outcome: {
      applied: "Applied",
      unchanged: "Unchanged",
      partial: "Partly applied",
      blocked: "Needs your decision",
    },
    userAction: {
      confirm_recreate: "You deleted this room before. Create it again?",
      resolve_conflict:
        "A room with this name already exists. SchoolX will not adopt it automatically.",
      request_ownership:
        "This room exists but you do not own it. Nothing was changed — ask an owner to apply it.",
    },
    renamed: "Renamed by a member",
    // Shown instead of a room name for `retired` items: the catalog entry is
    // gone, so no name survives anywhere. The internal key is shown beneath
    // it rather than in the name's place.
    unnamedItem: "Name unknown",
  },
} as const;
