import type { en } from "@/shared/i18n/locales/en";
import type { TranslationShape } from "@/shared/i18n/types";

export const ko = {
  app: {
    productName: "스쿨엑스",
    loading: {
      settingUpCommunity: "커뮤니티를 준비하고 있습니다…",
      switchingCommunity: "커뮤니티를 전환하고 있습니다…",
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
