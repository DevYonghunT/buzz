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
      voice: "Voice",
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
    adminRequired:
      "Only a community owner or admin can apply the default workspace. Ask an administrator to run it.",
    membershipUnavailable:
      "This relay does not publish community roles, so SchoolX cannot tell who may apply the default workspace. Enable relay membership (NIP-43) on the relay first.",
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
      // Adoption is anchored on who *created* the room, which never changes —
      // so this is not "you lack a permission someone could grant you". Being
      // made an owner or admin of the room will not unblock it. The only way
      // forward is for the person who created it to run this themselves.
      // See `CATALOG_SECURITY.md` §6.
      request_ownership:
        "This room was created by someone else. Nothing was changed — ask whoever created it to apply this item.",
    },
    renamed: "Renamed by a member",
    // Shown instead of a room name for `retired` items: the catalog entry is
    // gone, so no name survives anywhere. The internal key is shown beneath
    // it rather than in the name's place.
    unnamedItem: "Name unknown",
    // Labels the bare `item_key` shown under an unnamed (`retired`) item, so
    // it reads as a deliberate reference identifier rather than leftover
    // debug output.
    itemKeyLabel: "Reference ID: {{key}}",
    // Notes shown next to a ledger row's canvas step outcome. Keyed by the
    // same wire values as `CatalogStepStatus` (provenance.rs) — see
    // `canvasStepNoteKey` in WorkspaceCatalogSettingsCard.tsx. Only the two
    // values worth surfacing to an administrator get an entry; `done`,
    // `pending`, and `failed` render no note here.
    canvasStep: {
      // `StepStatus::Skipped`: the room already had content, so the starter
      // canvas was deliberately not written. The fact that matters to an
      // administrator is that nothing was lost — not the internal step name.
      skipped: "This room already had content, so it was left untouched.",
      // `StepStatus::Unrecognized`: a newer app version recorded this step
      // with a value this build does not know. Must not claim "kept" or
      // "written" — this build genuinely does not know which happened.
      unrecognized:
        "A newer version of the app recorded this step — this version can't show what happened.",
    },
  },
} as const;
