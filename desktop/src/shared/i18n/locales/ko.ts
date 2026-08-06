import type { en } from "@/shared/i18n/locales/en";
import type { TranslationShape } from "@/shared/i18n/types";

export const ko = {
  app: {
    productName: "스쿨엑스",
    loading: {
      settingUpCommunity: "커뮤니티를 준비하고 있습니다…",
      switchingCommunity: "커뮤니티를 전환하고 있습니다…",
    },
    selfHostedRelay: {
      cardTitle: "학교 릴레이를 직접 운영합니다",
      cardDescription:
        "학교가 직접 띄운 릴레이 주소로 연결합니다. 초대 코드가 필요 없습니다.",
      link: "직접 운영하는 릴레이가 있나요?",
      dialogTitle: "릴레이에 연결",
      dialogDescription:
        "학교가 운영하는 릴레이 주소를 입력하세요. 초대 코드는 필요 없습니다 — 처음 연결하는 관리자가 이미 소유자입니다.",
      placeholder: "ws://relay.our-school.example",
    },
    onboarding: {
      landing: {
        taglineTop: "우리 사람들, 에이전트, 프로젝트를",
        taglineBottom: "한곳에서",
        loading: "신원을 불러오는 중…",
        continueSetup: "설정 이어서 하기",
        // 「프라이빗키」로 부른다 — 사용 설명서와 같은 용어. 비밀번호가 아니라
        // 잃어버리면 복구할 수 없는 것이라는 감각을 주는 쪽을 택했다.
        createKey: "새 프라이빗키 만들기",
        useDifferentKey: "다른 프라이빗키로 바꾸기",
        useExistingKey: "이미 있는 프라이빗키 사용하기",
      },
      backup: {
        titleCreated: "내 프라이빗키가 만들어졌습니다",
        titleCreating: "프라이빗키를 만들고 있습니다",
        introContinue: " 지금 바로 계속하셔도 되고, ",
        optionsLink: "보관 방법 보기",
        introRestore: "에서 복구 방법을 확인하셔도 됩니다.",
        // 제품명을 문장에 넣지 않는다 — 화면의 워드마크는 라틴 문자
        // "SchoolX"인데 본문만 「스쿨엑스」로 쓰면 같은 화면에서 두 표기가
        // 부딪힌다. 주어를 생략하는 편이 한국어로도 자연스럽다.
        storage: {
          keychain:
            "프라이빗키를 시스템 키체인에 보관합니다. 키를 읽어야 할 때 컴퓨터가 암호를 물어볼 수 있습니다.",
          fileFallback:
            "시스템 키체인을 쓸 수 없어, 프라이빗키를 이 기기의 보호된 파일에 보관합니다.",
          device:
            "프라이빗키를 이 기기에 안전하게 보관합니다. 접근할 수 없게 될 때를 대비해 따로 백업해 두세요.",
        },
        introStorage: {
          keychain: "프라이빗키를 시스템 키체인에 보관하고 있습니다.",
          fileFallback:
            "시스템 키체인을 쓸 수 없어, 프라이빗키를 이 기기의 보호된 파일에 보관하고 있습니다.",
          device: "프라이빗키는 이 기기에 안전하게 보관되어 있습니다.",
        },
        storageTitle: {
          keychain: "시스템 키체인이 보호합니다",
          fileFallback: "기기의 보호된 저장소에 있습니다",
          device: "기기의 보호된 저장소가 보호합니다",
        },
        options: {
          title: "보관 방법",
          // 「비밀번호처럼 쓰인다」로 옮기지 않는다 — 비밀번호는 재발급이
          // 되지만 이 키는 안 된다. 그 차이가 이 화면의 요점이다.
          description:
            "프라이빗키는 내 계정을 여는 유일한 수단입니다. 사본을 안전한 곳에 보관하세요. 기억할 수 있는 비밀번호를 걸어 백업 파일로 만들 수 있습니다.",
        },
        neverShare:
          "프라이빗키를 다른 사람에게 보여주지 마세요. 이 키를 가진 사람은 누구나 나인 척할 수 있고 내 계정의 모든 것에 접근할 수 있습니다.",
        revealKey: "프라이빗키 보기",
        hideKey: "프라이빗키 가리기",
        copy: "클립보드에 복사",
        copied: "복사했습니다",
      },
      community: {
        chooseTitle: "커뮤니티에 참여하거나 새로 만들기",
        chooseDescription:
          "초대를 받아 참여하거나, 우리 커뮤니티를 새로 만들거나, 이미 쓰던 커뮤니티에 다시 연결합니다.",
        join: "커뮤니티에 참여하기",
        create: "커뮤니티 새로 만들기",
        existing: "이미 쓰던 커뮤니티가 있습니다",
        reconnectTitle: "쓰던 커뮤니티에 다시 연결",
        reconnectDescription:
          "어떤 역할이었는지 알려주시면 가장 빠른 길로 안내합니다.",
        owner: "제가 만든 커뮤니티입니다",
        memberOrAdmin: "멤버 또는 관리자입니다",
        memberEntryDescription:
          "커뮤니티 주소나 초대 링크를 입력하세요. 연결하면 원래 역할이 그대로 복구됩니다.",
        joinEntryDescription: "받으신 초대 링크나 커뮤니티 주소를 입력하세요.",
      },
      profile: {
        title: "어떻게 불러드릴까요?",
        description:
          "사람들과 에이전트에게 보일 이름입니다. 언제든지 바꿀 수 있습니다.",
        continue: "계속하기",
        createKey: "프라이빗키 만들기",
        saving: "프로필을 저장하는 중",
        skipForNow: "나중에 하기",
        continueWithoutSaving: "저장하지 않고 계속하기",
      },
      avatar: {
        title: "프로필 사진을 정해 주세요",
        description: "이미지나 이모지를 아바타로 고를 수 있습니다",
        addImage: "프로필 사진 추가",
      },
    },
    sidebar: {
      // 「받은 편지함」·「업무방」처럼 완전히 우리말로 옮기지 않는다. 이
      // 이름들은 화면 위치를 가리키는 고유 이름에 가깝고, 팀원끼리 "인박스
      // 봤어?"처럼 부르게 된다. 뜻이 갈리는 것만 우리말로 둔다.
      nav: {
        inbox: "인박스",
        pulse: "펄스",
        projects: "프로젝트",
        agents: "에이전트",
        workflows: "워크플로",
      },
      sections: {
        channels: "채널",
        forums: "포럼",
        directMessages: "개인 메시지",
      },
    },
  },
  settings: {
    sidebar: {
      groups: {
        personal: "개인",
        communities: "커뮤니티",
        app: "앱",
      },
      groupAriaLabel: "{{group}} 설정 항목",
      backToApp: "앱으로 돌아가기",
      checkingCommunityAccess: "초대 권한을 확인하고 있습니다…",
      communityAccessCheckFailed: "초대 설정을 확인하지 못했습니다.",
      communityAccessUnavailable:
        "초대 설정을 사용할 수 없습니다. 릴레이 복구가 진행 중일 수 있습니다.",
      tryAgain: "다시 시도",
    },
    sections: {
      appearance: "화면 및 언어",
      profile: "프로필",
      notifications: "알림",
      voice: "음성",
      experimental: "실험 기능",
      agents: "에이전트",
      channelTemplates: "템플릿",
      workspaceCatalog: "SchoolX 워크스페이스",
      compute: "컴퓨팅",
      shortcuts: "단축키",
      hostedCommunities: "호스팅 커뮤니티",
      communityMembers: "초대",
      moderation: "관리",
      customEmoji: "사용자 이모지",
      localArchive: "로컬 보관함",
      mobile: "모바일",
      updates: "업데이트",
    },
  },
  time: {
    today: "오늘",
    yesterday: "어제",
    justNow: "방금 전",
    // 한국어는 "5월 19일에"처럼 조사를 붙이지 않고 날짜만 둔다.
    onDate: "{{date}}",
  },
  appearance: {
    title: "화면 및 언어",
    description: "테마와 인터페이스 언어를 선택하세요.",
    mode: {
      system: "시스템",
      light: "라이트",
      dark: "다크",
    },
    accentColor: "강조 색상",
    threadLayout: {
      title: "스레드 레이아웃",
      focus: {
        label: "집중",
        description: "스레드를 채널 위에 전체 너비로 엽니다",
      },
      split: {
        label: "분할",
        description: "스레드를 채널 옆의 패널로 엽니다",
      },
    },
    language: {
      title: "인터페이스 언어",
      description: "이 기기의 메뉴와 조작 화면 언어를 변경합니다.",
      ko: "한국어",
      en: "영어",
    },
  },
  catalog: {
    title: "SchoolX 기본 워크스페이스",
    description:
      "SchoolX 표준 업무방을 만듭니다. 적용을 누르기 전에는 아무것도 생성되지 않습니다.",
    apply: "선택 항목 적용",
    applying: "적용하는 중…",
    adminRequired:
      "기본 워크스페이스는 커뮤니티 소유자나 관리자만 적용할 수 있습니다. 관리자에게 요청하세요.",
    membershipUnavailable:
      "이 릴레이는 커뮤니티 역할을 게시하지 않아, 누가 기본 워크스페이스를 적용할 수 있는지 판단할 수 없습니다. 릴레이에서 멤버십(NIP-43)을 먼저 켜야 합니다.",
    openWarningScope:
      "모든 로그인 사용자가 멤버가 아니어도 읽고 쓸 수 있습니다.",
    openWarningAgents:
      "관리형 에이전트는 명시적으로 추가된 경우에만 접근합니다.",
    decision: {
      create_or_recreate: "새로 만듭니다",
      resume: "이어서 진행합니다",
      no_change: "이미 적용됨",
      conflict: "확인이 필요합니다",
      retired: "더는 제공하지 않습니다",
      deleted: "이전에 삭제됨",
      adopted: "기존 방을 이어받습니다",
      not_owned: "다른 사람의 방입니다",
    },
    outcome: {
      applied: "적용 완료",
      unchanged: "변경 없음",
      partial: "일부만 적용",
      blocked: "확인이 필요합니다",
    },
    userAction: {
      confirm_recreate: "이전에 삭제한 방입니다. 다시 만들까요?",
      resolve_conflict:
        "같은 이름의 방이 이미 있습니다. SchoolX가 임의로 채택하지 않습니다.",
      request_ownership:
        "이 방은 다른 사람이 만들었습니다. 아무것도 바꾸지 않았습니다 — 만든 사람에게 적용을 요청하세요.",
    },
    recreate: {
      action: "다시 만들기",
      pending: "만드는 중…",
      // `request_ownership`에서만 나온다. 그 판정은 선점과 정상적인 공동
      // 관리를 구별하지 못하므로, 누르기 전에 결과를 먼저 말한다.
      ownedByOther:
        "다시 만들면 별개의 방이 새로 생기고, 기존 방은 그대로 남습니다. 되돌리려면 새 방을 직접 지워야 합니다.",
    },
    renamed: "멤버가 이름을 변경함",
    // `retired` 항목의 이름 자리에 들어간다 — catalog에서 빠져 이름이 남아
    // 있는 곳이 없다. 내부 키는 이름 대신이 아니라 그 아래에 따로 보여준다.
    unnamedItem: "이름을 알 수 없음",
    // 이름 없는(`retired`) 항목 아래 보이는 `item_key`에 붙는 라벨이다 —
    // 라벨이 없으면 영문 슬러그가 디버그 출력처럼 보인다.
    itemKeyLabel: "참조 ID: {{key}}",
    // ledger 행의 캔버스 단계 결과에 붙는 설명이다. `CatalogStepStatus`
    // (provenance.rs)와 같은 철자로 키를 삼는다 —
    // WorkspaceCatalogSettingsCard.tsx의 `canvasStepNoteKey` 참고. 관리자에게
    // 알려줄 가치가 있는 두 값만 있다 — `done`·`pending`·`failed`는 여기서
    // 별도 문구를 내지 않는다.
    canvasStep: {
      // `StepStatus::Skipped`: 방에 이미 내용이 있어 시작 캔버스를 일부러
      // 쓰지 않았다. 관리자에게 중요한 사실은 아무것도 사라지지 않았다는
      // 것이지, 내부 단계 이름이 아니다.
      skipped: "이 방에 이미 내용이 있어 그대로 두었습니다.",
      // `StepStatus::Unrecognized`: 더 최신 버전의 앱이 이 빌드가 모르는
      // 값을 기록했다. "지켰다"나 "썼다"라고 단정하면 안 된다 — 이 빌드는
      // 실제로 무슨 일이 있었는지 모른다.
      unrecognized:
        "더 최신 버전의 앱이 기록한 단계라 이 버전에서는 결과를 알 수 없습니다.",
    },
  },
} as const satisfies TranslationShape<typeof en>;
