import type { en } from "@/shared/i18n/locales/en";
import type { TranslationShape } from "@/shared/i18n/types";

export const ko = {
  app: {
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
      checkingCommunityAccess: "커뮤니티 접근 권한을 확인하고 있습니다…",
      communityAccessCheckFailed: "커뮤니티 접근 권한을 확인하지 못했습니다.",
      communityAccessUnavailable:
        "커뮤니티 접근 정보를 사용할 수 없습니다. 릴레이 복구가 진행 중일 수 있습니다.",
      tryAgain: "다시 시도",
    },
    sections: {
      appearance: "화면 및 언어",
      profile: "프로필",
      notifications: "알림",
      experimental: "실험 기능",
      agents: "에이전트",
      channelTemplates: "템플릿",
      compute: "컴퓨팅",
      shortcuts: "단축키",
      hostedCommunities: "호스팅 커뮤니티",
      communityMembers: "커뮤니티 접근",
      moderation: "관리",
      customEmoji: "사용자 이모지",
      localArchive: "로컬 보관함",
      mobile: "모바일",
      updates: "업데이트",
    },
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
} as const satisfies TranslationShape<typeof en>;
