export const en = {
  app: {
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
} as const;
