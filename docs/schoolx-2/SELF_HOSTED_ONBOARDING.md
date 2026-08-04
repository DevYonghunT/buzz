# 자체 호스팅 릴레이 온보딩 설계 (세션 G)

학교가 자기 릴레이를 돌린다는 SchoolX의 전제와, 앱이 그 릴레이로 들어가는
길을 안내하지 않는다는 사실 사이의 간극을 닫는다.

발견 경위와 재현은 [`BASELINE.md`](BASELINE.md)의 「실제 앱에서의 catalog
적용 확인」, 남은 과제 목록은
[`IMPLEMENTATION_HANDOFF.md`](IMPLEMENTATION_HANDOFF.md)에 있다.

## 1. 없는 것은 기능이 아니라 문이다

**먼저 확인해 둘 것: 릴레이 주소로 붙는 경로는 이미 동작한다.** 2026-08-04에
실제 앱에서 `ws://localhost:3000`을 입력해 붙었고, 그 세션에서 catalog 적용까지
전 경로가 돌았다. 코드도 그렇게 되어 있다.

- `InviteRedeemForm`의 `canSubmit`은 `normalizedRelayUrl !== null`만으로 참이다
  — 초대 코드가 없어도 제출된다.
- `normalizeRelayUrl`(`features/communities/relayProbe.ts`)은 `ws://`·`wss://`를
  받고 `http://`·`https://`를 각각 승격한다.
- 제출은 `WelcomeSetup`의 `startConnection(relayUrl)` →
  `communityOnboarding.start({ source: "first-community", relayUrl })`로 간다.
- 입력창 placeholder도 이미 "Invite link or community URL"이다.

**그러므로 이 설계는 새 연결 경로를 만들지 않는다.** 만드는 것은 그 경로로
가는 **문과 안내**다. 범위가 작다는 뜻이고, 그 사실을 먼저 못 박아 둔다 —
이것을 기능 개발로 잡으면 이미 있는 것을 다시 만들게 된다.

## 2. 학교 관리자가 어디서 잃는가

「Join or create a community」의 세 갈래를 자체 호스팅 관리자의 눈으로 따라간다.

| 고르는 것 | 그가 그렇게 고르는 이유 | 실제로 가는 곳 |
|---|---|---|
| Create a community | "우리 학교 커뮤니티를 만든다" | **Builderlab 호스팅 로그인** |
| I already have a community → I own it | "릴레이를 이미 띄웠으니 내 것이다" | **같은 Builderlab 다이얼로그** (`setIsHostedSignInOpen(true)`) |
| I already have a community → I'm a member | "나는 소유자인데?" — 고르지 않는다 | 릴레이 URL 입력 (**정답인데 안 고른다**) |
| Join a community | "초대받은 적 없다" — 고르지 않는다 | 릴레이 URL 입력 (**정답인데 안 고른다**) |

**정답 두 칸이 전부 「나는 초대받은 사람/구성원이다」로 이름 붙어 있다.**
자기 릴레이를 띄운 사람은 자신을 그렇게 부르지 않으므로 두 문 다 지나친다.
그리고 그가 자연스럽게 고르는 두 문은 **둘 다 외부 호스팅 서비스로** 나간다.

placeholder의 "or community URL"이 유일한 힌트인데, 그 문구를 보려면 이미 안
고를 문 안에 들어가 있어야 한다.

**초대로 우회할 수도 없다.** 초대는 owner 또는 admin만 발행할 수 있고
(`api/invites.rs`, NIP-98 필수, `X-Pubkey` 우회 없음) 아무도 안에 없는 새
릴레이에는 발행할 사람이 없다. 처음 들어가려면 이미 안에 있어야 한다.

## 3. upstream 병합 비용이 설계를 정한다

`WelcomeSetup.tsx`와 `HostedCommunityOnboarding.tsx`는 **SchoolX가 한 번도
건드리지 않은 upstream 원본**이다(foundation 이후 SchoolX 커밋 0건). 반면
upstream은 이 파일들을 계속 고친다 — 최근만 봐도 #2738(커뮤니티 관리 흐름
정리), #2862(join policy를 네이티브 네트워킹으로).

그래서 **화면을 재구성하지 않는다.** 카드 배치를 바꾸거나 페이지를 나누면
동기화마다 충돌이 나고, 그 충돌은 텍스트가 겹쳐 보이지 않는 종류일 수 있다
(`BASELINE.md`의 「git이 보지 못하는 동일-값 충돌」 참고).

**설계 원칙: 새 UI는 SchoolX 소유 파일에 두고, upstream 파일에는 호출 한 줄만
남긴다.**

## 4. 설계

### 4.1 새 컴포넌트 — `SelfHostedRelayEntry`

SchoolX 소유 파일 하나를 새로 만든다
(`desktop/src/features/communities/ui/SelfHostedRelayEntry.tsx`).

역할은 둘뿐이다.

- 자체 호스팅 관리자가 자기를 알아볼 문구를 보여준다.
- 누르면 기존 릴레이 URL 입력 화면으로 보낸다 — **새 폼을 만들지 않는다.**

`InviteRedeemForm`을 그대로 재사용한다. `variant`와 `onConnect`는 이미 그
용도로 있다.

### 4.2 두 막다른 곳에 문을 낸다

upstream 파일에 넣는 것은 각각 **한 줄**이다.

| 파일 | 넣는 것 |
|---|---|
| `WelcomeSetup.tsx`의 `existing` 페이지 | `<SelfHostedRelayEntry onSelect={...} />` — 세 번째 선택지 |
| `HostedCommunityOnboarding.tsx`의 다이얼로그 하단 | 같은 컴포넌트의 링크 형태 — 이미 들어와 버린 사람의 탈출구 |

두 번째가 중요하다. 오늘 실제로 막힌 지점이 그 다이얼로그였고, 거기서
빠져나갈 길이 없으면 앞의 안내를 놓친 사람은 그대로 갇힌다.

### 4.3 문구

「소유자/구성원」이 아니라 **「릴레이를 직접 운영하는가」**로 가른다. 그것이
자체 호스팅 관리자가 자기를 알아보는 유일한 표현이다.

| 키 | 한국어 | English |
|---|---|---|
| `selfHosted.title` | 학교 릴레이를 직접 운영합니다 | I run my own relay |
| `selfHosted.description` | 학교가 직접 띄운 릴레이 주소로 연결합니다. 초대 코드가 필요 없습니다. | Connect to a relay your school runs. No invite code needed. |
| `selfHosted.link` | 직접 운영하는 릴레이가 있나요? | Running your own relay? |
| `selfHosted.placeholder` | ws://relay.our-school.kr | ws://relay.our-school.kr |

"초대 코드가 필요 없습니다"를 명시한다 — 오늘 막힌 원인이 정확히 초대가
필요하다는 오해였다.

### 4.4 연결 실패를 설명한다

릴레이 주소를 처음 입력하는 사람은 오타·미기동·방화벽을 만난다. 지금은
`getJoinPolicy` 실패가 일반 오류로 뜬다. 최소한 **주소에 닿지 못한 것**과
**릴레이가 거절한 것**을 구별한다 — 전자는 사용자가 고칠 수 있고 후자는 아니다.

## 5. 검증 계획

| 검증 | 확인 대상 |
|---|---|
| Playwright | `existing` 페이지에 세 번째 선택지가 뜨고, 누르면 릴레이 URL 폼이 나온다 |
| | 호스팅 다이얼로그 안에서도 같은 탈출구가 보인다 |
| | 릴레이 URL만 넣고 제출하면 `startConnection`이 그 값으로 불린다 |
| 단위 | 문구 키가 en·ko 양쪽에 있다 (i18n parity) |
| 수동 | `just dev` + `ws://localhost:3000`으로 **안내만 보고** 끝까지 들어간다 |

마지막 줄이 이 작업의 진짜 완료 기준이다. 소스를 읽지 않은 사람이 화면만
보고 들어갈 수 있어야 한다.

## 6. 범위 밖

- **Builderlab 경로 자체는 건드리지 않는다.** 호스팅을 쓰는 배포도 그대로
  동작해야 한다.
- **릴레이 설치·운영 안내**(`BUZZ_REQUIRE_RELAY_MEMBERSHIP` 3종 묶음 등)는
  앱이 아니라 문서의 몫이다 — [`CONTRIBUTING.md`](../../CONTRIBUTING.md)에 이미
  적었다.
- **커뮤니티 생성 자체**는 만들지 않는다. 릴레이가 뜨면 커뮤니티는 이미
  있고(`seed-local-community.sh`, 배포 시엔 호스트 설정), 앱이 할 일은 붙는
  것뿐이다.
- 모바일은 별도다.
