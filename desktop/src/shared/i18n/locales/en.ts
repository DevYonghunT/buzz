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
    /**
     * Onboarding entry for schools that run their own relay.
     *
     * Worded around *running a relay*, not around owner/member. The two
     * choices that already reach this path are labelled "join with an invite"
     * and "I'm a member" — someone who just started their own relay does not
     * call themselves either, so they walk past both and pick one of the two
     * doors that lead to hosted sign-in instead. See
     * `docs/schoolx-2/SELF_HOSTED_ONBOARDING.md` §2.
     */
    selfHostedRelay: {
      cardTitle: "I run my own relay",
      cardDescription:
        "Connect to a relay your school runs. No invite code needed.",
      link: "Running your own relay?",
      dialogTitle: "Connect to your relay",
      // "No invite code" is the load-bearing sentence: an invite can only be
      // minted by an existing owner or admin, so on a brand-new relay there is
      // nobody to ask — believing you need one is the dead end itself.
      dialogDescription:
        "Enter the address of the relay your school runs. You do not need an invite code — the first administrator to connect is already an owner.",
      placeholder: "ws://relay.our-school.example",
    },
    /**
     * Machine onboarding. Kept inside `app` rather than a namespace of its own:
     * a namespace present in `en` and missing from `ko` renders every key in it
     * as a raw path for Korean users, and `fallbackLng` does not rescue it.
     */
    onboarding: {
      landing: {
        // Two lines because the design breaks the sentence deliberately; the
        // Korean break lands in a different place than a naive split would.
        taglineTop: "Your people, your agents, your projects —",
        taglineBottom: "all in one place.",
        loading: "Loading identity…",
        continueSetup: "Continue setup",
        createKey: "Create a new identity key",
        useDifferentKey: "Use a different key instead",
        useExistingKey: "Use an existing key",
      },
      backup: {
        titleCreated: "Your unique identity key has been created",
        titleCreating: "Creating your identity key",
        /**
         * The intro is one sentence with a link inside it. Split into three
         * pieces rather than interpolated, because Korean puts the particle
         * *after* the link text — a single string with a placeholder cannot
         * express that without stranding the particle in English word order.
         */
        introContinue: " You can continue now, or ",
        optionsLink: "review backup options",
        introRestore: " for ways to restore your account.",
        storage: {
          keychain:
            "SchoolX keeps your identity key in your system keychain. Your computer may ask for your password when SchoolX needs to read the key.",
          fileFallback:
            "Your system keychain wasn’t available, so SchoolX keeps your identity key in a private file on this device.",
          device:
            "SchoolX keeps your identity key protected on this device. Make a separate backup in case you lose access.",
        },
        /** Shorter storage line, shown in the intro sentence above the key. */
        introStorage: {
          keychain: "SchoolX keeps your identity key in your system keychain.",
          fileFallback:
            "SchoolX keeps your identity key in a private file on this device because the system keychain wasn’t available.",
          device: "Your identity key is protected on this device.",
        },
        storageTitle: {
          keychain: "Protected by your system keychain",
          fileFallback: "Stored in private device storage",
          device: "Protected in private device storage",
        },
        options: {
          title: "Backup options",
          description:
            "Your identity key works like a password for your SchoolX account. Keep a copy somewhere safe. You can create a backup file and lock it with a password you can remember.",
        },
        neverShare:
          "Never share your private key. Anyone with this key can impersonate you and access everything in your account.",
        revealKey: "Reveal private key",
        hideKey: "Hide private key",
        copy: "Copy to clipboard",
        copied: "Copied to clipboard",
      },
      community: {
        chooseTitle: "Join or create a community",
        chooseDescription:
          "Join with an invite, create your own community, or reconnect one you already have.",
        join: "Join a community",
        create: "Create a community",
        existing: "I already have a community",
        reconnectTitle: "Reconnect to your community",
        reconnectDescription:
          "Tell us your role so we can find the fastest way back in.",
        owner: "I own the community",
        memberOrAdmin: "I’m a member or admin",
        memberEntryDescription:
          "Enter the community URL or an invite link. Your role will be restored when you connect.",
        joinEntryDescription:
          "Enter the invite link or community URL you received.",
      },
      profile: {
        title: "What should we call you?",
        description:
          "Pick the name people and agents will see in SchoolX. You can change it anytime.",
        continue: "Continue",
        createKey: "Create an identity key",
        saving: "Saving profile",
        skipForNow: "Skip for now",
        continueWithoutSaving: "Continue without saving",
      },
      avatar: {
        title: "Next, add a display image",
        description: "Choose an image or emoji as your avatar",
        addImage: "Add a display image",
      },
    },
    sidebar: {
      nav: {
        inbox: "Inbox",
        pulse: "Pulse",
        projects: "Projects",
        agents: "Agents",
        workflows: "Workflows",
      },
      sections: {
        channels: "Channels",
        forums: "Forums",
        directMessages: "Direct messages",
      },
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
    recreate: {
      action: "Create it again",
      pending: "Creating…",
      // Shown only for `request_ownership`. That verdict cannot tell a squatter
      // apart from an ordinary co-administrator, so the consequence is stated
      // before the button rather than after.
      ownedByOther:
        "Creating it again makes a new, separate room and leaves the existing one alone. Undoing that means deleting the new room yourself.",
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
