# SchoolX 내부 시각 체계 V1 구현 인계

기준일: **2026-08-25**

문서 상태: **2026-08-26 SchoolX first-party theme + core shell V1 구현 완료**.
아래 §1 이후 본문은 착수 당시의 negative control과 결정 기록이므로 현재 상태로
덮어쓰지 않고 보존한다. 구현 결과와 실행 증거는 §0을 기준으로 읽는다.

Repository 기준:

- branch: `codex/schoolx-release-readiness-20260825`
- 준비 문서 작성 전 HEAD: `f713de57dc07eabf53200d45c26b86b87a3ff426`
- canonical brand source: [`brand/`](../../brand)

읽는 순서는 다음과 같다.

1. 현재 구현 범위와 완료 기준: 이 문서
2. 확정 색상과 제품 구조: [`SCHOOLX_CODE_DESIGN.md` §6](SCHOOLX_CODE_DESIGN.md#6-브랜드와-시각-체계)
3. 제품 문자열과 내부 호환 namespace 구분:
   [`PRODUCT_IDENTITY.md`](PRODUCT_IDENTITY.md)
4. 데스크톱 규칙과 검증 명령: [`AGENTS.md`](../../AGENTS.md)
5. 배포 아이콘과 signed artifact의 별도 상태:
   [`SESSION_HANDOFF_20260825_CODE_RELEASE_READINESS.md`](SESSION_HANDOFF_20260825_CODE_RELEASE_READINESS.md)

## 0. 2026-08-26 완료 기록

이번 slice는 first-party `buzz`/`buzz-dark`의 공개 presentation만 SchoolX V1으로
교체했다. internal theme ID, `buzz-*` localStorage, `data-buzz-*`, Shiki
GitHub Light/Dark mapping, explicit third-party theme와 accent, route/Tauri/relay/Git
authority는 유지했다.

- [`brand/`](../../brand)을 canonical source로 삼아 SVG/public PNG/manifest를
  생성·검증하며 favicon, Nostr bind와 Mobile QR 중앙 icon을 같은 mark로 맞췄다.
- Light/Dark complete semantic map과 Appearance label/preview가 한 resolver를 쓰고,
  first-party accent는 SchoolX mapping에 고정된다. third-party adaptive theme와
  accent picker는 기존 동작을 유지한다.
- first-party cache revision과 full-map fingerprint를 inline prepaint와 mounted
  provider 양쪽에서 검증한다. fresh Light/Dark의 최초 html/body backing과 root
  `--background`/`--foreground`도 bundle-blocked E2E로 고정했다.
- Home/core shell은 flat·opaque surface를 사용하고 boot/blocking/community-switch
  gate는 accessible status text와 정적 SchoolX mark를 제공한다. 새 brand animation,
  route, command 또는 authority는 추가하지 않았다.

자동 검증 결과:

- Biome, TypeScript, px-text guard, asset generator `--check`, E2E build: 통과
- desktop test: 4,061 통과, 실패 0; brand asset contract: 4/4 통과;
  focused SchoolX theme contract: 14/14 통과
- 지정 smoke 묶음: 37/37 통과; SchoolX Code `entry points|800x500`: 2/2 통과;
  community-switch appearance/static mark 반복: 2/2 통과
- file-size ratchet은 known debt 때문에 예상대로 exit 1이지만 기준 19개 tuple과
  exact match이며 신규 path와 기존 tuple line 증가가 모두 0이다. `AppShell.tsx`는
  999줄로 비변경, `SettingsPanels.tsx`는 990줄이다.
- `git diff --check` 통과. 기존 dirty 파일을 포함해 staged path는 0개이며 commit과
  push는 수행하지 않았다.

설치파일 제작을 위해 만든 별도 clean worktree에서는 §12의 사용자 변경을 복사하지
않았다. 따라서 같은 base의 file-size ratchet은 원본 dirty snapshot의 19개가 아니라
17개 known tuple을 출력하며, 빠진 두 tuple은 제외 대상인
`managed_agents/runtime.rs`와 `managed_agents/runtime/tests.rs`다. Visual path의 신규
violation과 기존 tuple 증가는 여전히 0이다. 이 격리본에서 desktop test 4,062/4,062,
지정 smoke 37/37, SchoolX Code focused 2/2를 다시 통과했다. 위 staged/commit 문장은
최초 완료 기록 당시의 상태이며, 설치파일 제작용 sign-off commit은 별도 이력으로
남긴다.

필수 screenshot은 모두 서로 다른 SHA-256을 가진다.

```text
68f5d9ff2b45ae2baa73844af9b24a9c09645be1d34b724046190dc904cbffb3  01-home-light.png
fc4e0444026572970b53392bf948bae9fd7cb20de62946a499cf090221c1e533  02-home-dark.png
3258837ad034796672bcacc9f98518d168c62a016997d45980ead7488a5d29b7  03-appearance-system.png
1036367ee8182a8e0aa5a407380d2707231c1f693b86cf816af0425421608485  04-project-card-code-entry.png
2b8f08f9b55dc77ba108b36591a7a11c2cfb5fb52bfe131b51932b5523dc0bf7  05-project-detail-code-entry.png
8f6e0c85edcdd85c970fcf260a5bedb5711ee93466045671cb8b40588a144c82  06-code-dark-reduced-150.png
3374f8ce747c4ae03615ce3f96f7fc28342ac2c39bc43c123fa164574037662c  07-mobile-pairing-card.png
```

Browser mock으로 증명할 수 없는 실제 Tauri macOS Light/Dark cold boot, live system
appearance, Reduce Transparency, Increase Contrast와 실제 device QR scan은 아직 수동
확인하지 않았다. 이를 native 완료로 주장하지 않는다. First-run onboarding과 §10의
legacy branded surface도 의도적으로 후속 slice에 남아 있다.

## 1. 현재 판정

새 SchoolX 디자인은 **배포 표면에는 적용됐지만 웹뷰 내부 UI에는 아직 적용되지
않았다**. 따라서 현재 상태를 “브랜딩 완료” 또는 “미적용” 중 하나로만 표현하면
둘 다 부정확하다.

| 표면 | 현재 상태 | 근거 |
|---|---|---|
| Dock/app bundle, tray, DMG | SchoolX 적용 | `ddcda9c6f`와 `brand/schoolx-*` |
| 메인 앱 Light/Dark theme | 미적용 | `buzz`/`buzz-dark`가 GitHub Light/Dark에서 runtime CSS vars를 파생 |
| Appearance 공개 label/preview | 미적용 | 사용자에게 `Buzz`/`Buzz Dark`와 기존 gradient가 보임 |
| boot/community switch | 미적용 | `BuzzMark`, `FuzzyLogo`, `FlappingBee` 사용 |
| Nostr bind/Mobile QR 내부 app icon | 미적용 | 오래된 `desktop/public/app-icon@2x.png`, `app-icon@3x.png` 사용 |
| first-run onboarding | 미적용 | chartreuse/light-blue 배경, bee swarm, Buzz mark/wordmark 사용 |
| SchoolX Code 구조와 동작 | 구현됨, 이번 slice에서 비변경 | 프로젝트 진입점, task/timeline/Changes/terminal은 기존 회귀를 유지 |

2026-08-25에 `/Applications/SchoolX.app` 0.5.3을 직접 확인했을 때 OS 아이콘은
SchoolX였지만 메인 화면과 Settings → Appearance는 기존 Buzz 시각 체계였다. 이
관측은 UI negative control이며 해당 설치본의 release provenance나 최신 source
artifact를 증명하지 않는다.

Source에서도 같은 경계가 확인된다.

- `desktop/src`에는 확정 팔레트의 여섯 hex나 `schoolx-mark.svg` import가 없다.
- [`theme-loader.ts`](../../desktop/src/shared/theme/theme-loader.ts)는 first-party
  alias를 GitHub Light/Dark Shiki theme에 매핑한다.
- [`ThemeProvider.tsx`](../../desktop/src/shared/theme/ThemeProvider.tsx)는 그
  색으로 만든 vars를 root inline style로 덮어쓴다. `theme.css`만 바꾸면 실제
  화면은 바뀌지 않는다.
- [`useThemePreviewVars.ts`](../../desktop/src/shared/theme/useThemePreviewVars.ts)는
  preview를 별도로 파생하므로 실제 theme와 함께 변경하지 않으면 picker가 거짓
  미리보기를 보여 준다.
- `buzz-theme-cache`에는 schema/revision이 없어 업그레이드 첫 프레임에 예전
  GitHub/Buzz palette가 다시 칠해질 수 있다.
- [`components.css`](../../desktop/src/shared/styles/globals/components.css)의
  onboarding 전용 색과 패턴은 global semantic theme의 영향을 받지 않는다.

## 2. 이번 구현의 단 하나의 slice

### SchoolX first-party theme + core shell

다음 세션은 아래 여섯 항목을 한 묶음으로 완료한다.

1. 기존 first-party Light/Dark alias가 실제 SchoolX semantic palette를 적용한다.
2. Appearance에서 공개 이름과 preview가 `SchoolX` / `SchoolX Dark`로 보인다.
3. fresh profile, 기존 first-party 선택, explicit third-party 선택의 저장 동작을
   보존하면서 stale first-party cache를 안전하게 무효화한다.
4. 메인 shell의 기존 yellow/blue gradient와 wallpaper-dependent translucency를
   flat, opaque SchoolX surface로 교체한다.
5. cold boot, blocking load, community switch의 bee/fuzzy logo를 canonical
   SchoolX mark 기반의 정적 상태 표시로 교체한다.
6. favicon과 Nostr bind/Mobile QR 중앙 app icon을 canonical SchoolX mark에서
   생성한 local asset으로 교체한다.

이 slice가 끝나면 기존 사용자가 가장 자주 보는 Home, Settings, Projects,
SchoolX Code가 새 palette를 실제로 사용한다. 그러나 first-run onboarding은 아직
이전 디자인이므로 **앱 내부 전체 rebrand 완료**라고 표현하지 않는다.

## 3. 구현 전 decision gate — 이 문서에서 고정

다음 결정은 V1 구현자가 다시 넓히거나 뒤집지 않는다.

| 질문 | V1 결정 | 이유 |
|---|---|---|
| 새 theme ID를 만들 것인가 | **아니오**. 내부 `buzz`/`buzz-dark` 유지 | 저장 설정, system pair, Shiki, cache, test ID migration을 불필요하게 넓히지 않음 |
| `buzz-*` localStorage/data attribute를 rename할 것인가 | **아니오** | [`PRODUCT_IDENTITY.md`](PRODUCT_IDENTITY.md)의 내부 namespace 호환 계약 |
| 기존 third-party theme를 SchoolX로 강제할 것인가 | **아니오** | 사용자가 명시적으로 고른 appearance를 보존 |
| first-party sidebar vibrancy/gradient를 유지할 것인가 | **아니오**. V1은 opaque/flat | exact brand 색, 대비, Reduce Transparency, deterministic screenshot을 우선 |
| 새 logo animation을 만들 것인가 | **아니오**. 정적 mark | 요청되지 않은 장식 motion을 추가하지 않고 Reduce Motion을 기본 충족 |
| horizontal wordmark를 webview에 넣을 것인가 | **아니오**. V1은 mark-only | canonical logo의 `<text>`가 설치 font에 따라 달라짐. outlined lockup 결정 전 보류 |
| `buzz-agent` runtime/vendor icon을 SchoolX로 바꿀 것인가 | **아니오** | 앱 브랜드가 아니라 runtime identity이며 `buzz-agent`는 의도적 잔존 이름 |
| onboarding bee swarm을 X mark로 기계 치환할 것인가 | **아니오** | 수십 개 X가 떠다니는 잘못된 결과가 됨. 별도 backdrop 설계가 필요 |
| layout, 정보 구조, 기능 동작도 바꿀 것인가 | **아니오** | 이번 slice는 semantic visual layer와 brand-owned core surface만 다룸 |

## 4. 호환·저장 public contract

### 4.1 반드시 유지할 값

- persisted theme IDs: `buzz`, `buzz-dark`
- theme storage: `buzz-theme`
- synchronous cache key: `buzz-theme-cache`
- accent storage: `buzz-accent-color`
- follow-system storage: `buzz-follow-system`
- DOM/test/internal compatibility marker: 기존 `data-buzz-*`
- Shiki mapping: `buzz → github-light`, `buzz-dark → github-dark`
- route, Tauri command/event, relay wire, DB schema, SchoolX Code authority: 변경 없음

Shiki mapping은 **코드 syntax highlighting 전용**이다. 앱 chrome와 semantic vars가
GitHub palette를 계속 써야 한다는 뜻이 아니다.

Theme preference는 현재 community별 설정이 아니라 WebView origin의 앱 전역 로컬
설정이다. community switch, relay sync, Nostr event에 새 theme state를 추가하지
않는다.

### 4.2 사용자 상태 migration

| stored theme | stored follow-system | V1 결과 |
|---|---|---|
| 없음 | 없음 | fresh profile: `buzz`를 선택하고 OS에 따라 SchoolX Light/Dark effective pair 적용 |
| 없음 | `false` | fallback `buzz`를 fixed SchoolX Light로 적용. 부분 storage 상태를 임의로 OS 추종으로 바꾸지 않음 |
| `buzz`/`buzz-dark` | `false` | 같은 ID와 fixed Light/Dark presentation 유지 |
| `buzz`/`buzz-dark` | `true` | stored ID는 유지하고 effective pair는 현재 OS mode가 결정 |
| explicit third-party | 없음 또는 `false` | 선택과 accent를 그대로 보존. follow key가 없고 theme key가 있으면 기존처럼 fixed |
| paired third-party | `true` | 기존 pair와 OS 추종 동작 보존 |
| unpaired third-party | `true` | 기존 resolver처럼 selected theme 유지. 이 비정상/주입 상태를 SchoolX로 강제 reset하지 않음 |
| legacy `light` | 기존 follow 값 | 기존대로 `catppuccin-latte`로 migration한 뒤 같은 follow 규칙 적용 |
| legacy `dark` 또는 `system` | 기존 follow 값 | 기존대로 `houston`으로 migration한 뒤 같은 follow 규칙 적용 |
| invalid/unsupported ID | 기존 follow 값 | selected theme만 기존 fallback으로 복구하고 follow 값은 보존 |

다른 theme를 명시적으로 선택한 사용자까지 강제로 SchoolX로 reset하면 안 된다.
사용자에게 새 디자인을 기본으로 보여 주는 대상은 fresh profile과 현재 first-party
alias 사용자다.

### 4.3 cache revision

`buzz-theme-cache` key는 유지하되 payload에 명시적인 first-party palette revision을
둔다. Revision literal은 TypeScript와 `index.html`에 중복될 수밖에 없으므로
source-consistency test가 두 값을 byte-for-byte 비교한다. 예전 revision 또는
revision 없는 `buzz`/`buzz-dark` cache는
[`index.html`](../../desktop/index.html) prepaint와 `ThemeProvider.applyCachedVars()`
양쪽에서 모두 거부해야 한다.

- old first-party cache의 vars를 한 frame도 root에 복사하지 않는다.
- third-party cache는 단지 이름에 `buzz`가 없다는 이유로 폐기하지 않는다.
- first-party stored selection/follow-system에서 계산한 effective theme와 cache의
  `themeName`이 다르면 cache를 적용하지 않는다. 특히 OS mode가 앱을 닫아 둔
  동안 바뀐 경우 이전 first-party mode를 먼저 칠하지 않는다. Third-party pair의
  prepaint 동작까지 이 slice의 부수 refactor로 넓히지 않는다.
- 새 first-party vars와 revision은 같은 write에서 저장한다.
- 새 brand-only var가 생기면 non-first-party theme로 나갈 때 모두 overwrite 또는
  clear해 stale inline var가 남지 않게 한다.
- inline prepaint와 React path가 서로 다른 revision 규칙을 갖지 않도록 exact
  regression을 둔다.

## 5. 시각 contract

### 5.1 canonical palette

| 이름 | 값 | V1 의미 |
|---|---|---|
| Parchment | `#F4EDDD` | Light canvas, Dark foreground |
| Pine | `#355649` | Light navigation/default primary, Dark selection tint |
| Terracotta | `#B85A3C` | Light의 명시적 실행·주의 CTA에 제한 |
| Terracotta Dark | `#D97958` | Dark action/focus 후보 |
| Ink | `#1F2937` | Light foreground, Dark canvas |
| Warm Gold | `#D7A94B` | pending/highlight/focus |
| Sage | `#7F967A` | success/secondary |

Exact anchor와 mode별 complete semantic var map은 새
`desktop/src/shared/theme/schoolx-theme.ts` 한 곳에서 만든다. supporting card,
popover, muted, border elevation은 그 모듈의 pure derivation만 허용하고 feature
component에 hex를 흩뿌리지 않는다.

최소 mapping은 다음을 만족한다.

- Light canvas/card의 가장 큰 면적은 Parchment 계열이다.
- Light navigation과 default primary는 Pine 계열이며 foreground는 Parchment다.
- Dark canvas의 가장 큰 면적은 Ink이고 순수 검정을 쓰지 않는다.
- Dark selection에 Pine을 쓸 때 Pine/Ink 경계만으로 상태를 전달하지 않는다.
  foreground, border 또는 icon 중 하나로 충분한 추가 대비를 준다.
- Light action의 Terracotta foreground는 white다. Parchment는 일반 text 대비에
  실패하므로 사용하지 않는다.
- 실행·주의 CTA와 destructive/error는 같은 의미가 아니다. Terracotta를 기존
  `--destructive`에 기계적으로 매핑하지 않는다.
- Dark action의 `#D97958` foreground는 Ink다.
- Gold와 Sage는 상태를 보조하며 한 view의 주 accent를 서로 경쟁시키지 않는다.
- code/terminal의 syntax token은 GitHub Light/Dark Shiki mapping을 유지한다.

### 5.2 고정 contrast gate

다음 수치는 pure test로 고정한다.

| 조합 | contrast | 판정 |
|---|---:|---|
| Ink / Parchment | `12.59:1` | normal text 통과 |
| Parchment / Pine | `6.98:1` | normal text 통과 |
| White / Terracotta | `4.60:1` | normal text 통과 |
| Ink / Terracotta Dark | `4.77:1` | normal text 통과 |
| Ink / Warm Gold | `6.76:1` | normal text 통과 |
| Ink / Sage | `4.58:1` | normal text 통과 |
| Parchment / Terracotta | `3.94:1` | normal text 금지 |
| Parchment / Sage | `2.75:1` | normal text 금지 |
| Pine / Ink | `1.80:1` | focus/selection 경계 단독 사용 금지 |

Normal text는 최소 `4.5:1`, focus ring과 non-text state boundary는 최소 `3:1`을
만족한다. 자동 axe dependency는 현재 없으므로 이 문서에서 “axe 통과”를 완료
근거로 쓰지 않는다.

### 5.3 core shell

- 기존 yellow/blue, multicolor gradient와 glow를 SchoolX theme에 남기지 않는다.
- sidebar/content layout, splitter, toolbar, traffic lights, keyboard shortcut은
  변경하지 않는다.
- opaque surface로 native wallpaper와 무관하게 같은 computed color가 나온다.
- `prefers-reduced-motion`에서는 새 looping brand motion이 없어야 한다.
- Reduce Transparency에서도 정보 손실이 없어야 한다. V1 first-party surface가
  opaque이므로 투명 fallback에 의존하지 않는다.
- Increase Contrast에서는 focus와 selection이 색 하나에만 의존하지 않게 한다.
- readable text는 기존 rem token만 사용하고 px/arbitrary text size를 추가하지
  않는다.
- icon-only control을 추가한다면 기존 Radix primitive와 명확한 `aria-label`을
  사용한다. 이번 slice는 새 interaction primitive를 만들지 않는다.

### 5.4 brand asset contract

`brand/`가 유일한 원본이다. WebView용 파일은
[`generate-schoolx-brand-assets.sh`](../../desktop/scripts/generate-schoolx-brand-assets.sh)가
생성하는 checked-in derived asset으로 둔다.

- network/third-party image URL 금지
- raw SVG `dangerouslySetInnerHTML` 금지
- local `<img>` wrapper를 우선 사용
- decorative mark는 `alt=""`와 `aria-hidden`; 의미 있는 product mark는 localized
  accessible name 사용
- generator는 public SVG byte copy와 PNG, source hash가 든 manifest를 함께 쓴다.
  Cross-platform Node test가 canonical SVG hash, public copy byte equality, manifest,
  PNG hash/metadata를 검사한다. macOS generator 재실행은 regenerate-and-compare
  검증이며 Linux CI가 `sips`를 실행할 필요는 없다.
- generator에 non-writing `--check` mode를 추가한다. macOS에서는 temp output과
  checked-in derived asset을 비교하고 drift 시 실패한다.
- `/buzz.svg`는 mock markdown fixture가 참조하므로 V1에서 무조건 삭제하지 않는다.
  favicon reference만 SchoolX asset으로 바꾼다.
- 기존 `/app-icon@2x.png`, `/app-icon@3x.png`는 각각 exact 112×112/168×168로
  다시 생성한다. 둘은 완전 불투명한 Parchment plate 위에 여백을 둔 SchoolX
  mark여야 한다. `StyledQrCode`가 center image 뒤에 foreground rect를 그리므로
  transparent PNG를 쓰면 Pine/Ink가 검정 위에서 사라진다.
- Nostr bind와 Mobile QR는 같은 product icon을 쓰되 QR의 기존 matrix/오류정정
  구조 test를 유지하고 실제 device scan을 수동 확인한다. 새 decoder dependency는
  이 visual slice에 추가하지 않는다.

## 6. fault matrix

| 상황 | 기대 결과 | 필수 증거 |
|---|---|---|
| fresh Light | 첫 paint부터 Parchment/SchoolX Light | prepaint unit + E2E computed vars |
| fresh Dark | 첫 paint부터 Ink/SchoolX Dark | OS media seed E2E |
| old unversioned `buzz` cache | old GitHub/yellow-blue vars를 적용하지 않음 | sentinel cache regression |
| valid new first-party cache | 동기 prepaint와 mounted theme가 일치 | cache round-trip test |
| cache theme와 stored/system effective theme 불일치 | cache를 건너뛰고 현재 effective mode로 첫 paint | prepaint mismatch E2E |
| explicit GitHub/Catppuccin 등 | 선택과 accent picker 동작 보존 | E2E theme switch |
| invalid cache JSON/shape | prepaint는 safe fallback; React mount 뒤에는 stored third-party를 포함한 실제 preference 적용 | pure test + boot smoke |
| Light↔Dark rapid switch | 마지막 선택만 적용, stale async result 없음 | rapid-toggle E2E |
| Follow System live change | reload 없이 paired alias 전환 | media change E2E |
| brand→third-party | first-party-only inline vars/attributes leak 없음 | computed var regression |
| native vibrancy IPC failure | opaque SchoolX surface 유지 | mocked rejection test |
| community switch | app-global appearance 유지 | existing switch path smoke |
| reduced motion | static mark, looping brand animation 없음 | reduced-motion E2E |
| 800×500 + 150% text | Settings/Projects/Code에 document overflow나 잘린 CTA 없음 | focused layout E2E |
| generated mark missing/drift | build/test가 조용히 fallback하지 않고 실패 | asset contract test |

Native cold boot의 최초 backing은 현재 고정 RGB `(17, 21, 24)`다. Light mode에서
실제 dark flash가 관측되면 [`desktop/src-tauri/src/lib.rs`](../../desktop/src-tauri/src/lib.rs)의
기존 backing 함수 안에서만 OS appearance에 맞춰 Parchment/Ink를 선택한다. 새 Tauri
command나 product authority를 만들지 않는다.

## 7. UI/E2E 완료 기준

V1은 아래를 모두 만족해야 완료다.

### 7.1 기능·호환 assertion

- internal persisted ID는 여전히 `buzz`/`buzz-dark`다.
- Appearance theme picker의 공개 label은 `SchoolX`/`SchoolX Dark`이며 그 picker
  안에는 visible `Buzz` theme label이 없다.
- applied root vars와 preview tile vars가 같은 SchoolX resolver 결과다.
- fresh/system/stored first-party/explicit third-party matrix가 §4.2와 같다.
- stored accent가 first-party Pine/Terracotta mapping을 덮어쓰지 않는다.
- third-party theme에서는 기존 accent picker와 adaptive vars가 변하지 않는다.
- Shiki resolver는 계속 GitHub Light/Dark를 반환한다.
- Home, Settings, Projects, SchoolX Code의 route와 keyboard interaction이 유지된다.
- Nostr bind와 Mobile QR 중앙 이미지가 local SchoolX app icon이다.
- boot/loading/community switch는 static SchoolX mark와 기존 accessible status text를
  제공한다.

### 7.2 visual evidence

기존 golden pixel snapshot 관례를 새로 만들지 않는다. Exact computed CSS/geometry
assertion과 검토용 screenshot을 함께 남긴다.

필수 screenshot:

1. 1280×720 Home shell — SchoolX Light
2. 1280×720 Home shell — SchoolX Dark
3. Appearance system picker — SchoolX pair label/preview
4. locator-scoped Project card SchoolX Code 진입점
5. locator-scoped Project detail SchoolX Code 진입점
6. 800×500 SchoolX Code — Dark + reduced motion
7. Mobile pairing card의 SchoolX icon

모든 screenshot 직전에 shared `waitForAnimations(page)`를 호출한다. 결과는
`shasum -a 256`으로 비교해 서로 다른 상태의 hash가 모두 달라야 한다. Repository
media endpoint나 외부 image host를 쓰지 않는다.

### 7.3 접근성·Mac 수동 확인

- theme tile은 `aria-pressed`, keyboard focus, Return/Space activation을 유지한다.
- focus ring `3:1`, normal text `4.5:1`을 만족한다.
- reduced motion에서 brand animation은 정지하는 수준이 아니라 존재하지 않는다.
- 새 theme spec이 `buzz:text-scale=1.5`를 seed한 Settings, Projects, SchoolX Code
  case를 각각 가진다. 단순 document overflow 검사만 하지 않고 핵심 CTA의
  bounding box, visible state와 keyboard reachability를 단언한다.
- 실제 Tauri macOS에서 Light/Dark cold boot, live system appearance change, Reduce
  Transparency, Increase Contrast를 수동 확인한다.

Browser mock은 native backing, vibrancy 또는 macOS accessibility preference 전달을
증명하지 않는다. 이 수동 확인 없이 “native appearance 완료”라고 쓰지 않는다.

## 8. 예상 수정 파일

구현 중 실제 필요가 없는 파일은 건드리지 않는다. 예상 범위는 다음과 같다.

### Theme source

- 새 `desktop/src/shared/theme/schoolx-theme.ts`
- 새 `desktop/src/shared/theme/schoolx-theme.test.mjs`
- `desktop/src/shared/theme/ThemeProvider.tsx`
- `desktop/src/shared/theme/theme-loader.ts`
- `desktop/src/shared/theme/useThemePreviewVars.ts`
- `desktop/src/shared/theme/ThemePreviewFrame.tsx`
- `desktop/src/shared/styles/globals/theme.css`
- `desktop/src/features/settings/ui/SettingsPanels.tsx` — display helper 연결만

`SettingsPanels.tsx`는 현재 990줄이다. palette/cache/label 로직을 이 파일에 넣지
않는다. 변경이 단순 import/호출을 넘으면 `ThemeSettingsCard.tsx`로 먼저 분리한다.

### Brand/prepaint source

- `desktop/scripts/generate-schoolx-brand-assets.sh`
- 새 `desktop/src/shared/ui/schoolx-brand/SchoolXMark.tsx`
- 새 `desktop/src/shared/ui/schoolx-brand/schoolx-brand-assets.test.mjs`
- `desktop/src/app/App.tsx`
- `desktop/index.html`
- `desktop/public/brand/`의 generated SchoolX mark asset
- `desktop/public/brand/manifest.json` — canonical source와 derived asset hash/metadata
- regenerated `desktop/public/app-icon@2x.png`
- regenerated `desktop/public/app-icon@3x.png`
- 조건부 `desktop/src-tauri/src/lib.rs` — §6의 visible backing flash가 재현될 때만

`desktop/src/app/AppShell.tsx`는 현재 999줄이므로 이번 visual slice에서 손대지
않는다. selector rename이나 layout cleanup이 필요해 보여도 별도 refactor로 남긴다.

### Tests

- 새 `desktop/tests/helpers/contrast.ts`
- `desktop/tests/e2e/buzz-theme-screenshots.spec.ts`를
  `schoolx-theme-screenshots.spec.ts`로 rename/update
- `desktop/tests/e2e/boot-splash.spec.ts`
- `desktop/tests/e2e/mobile-pairing-qr.spec.ts`
- `desktop/tests/e2e/nostr-bind.spec.ts`
- `desktop/playwright.config.ts`

### Documentation on completion

- 이 문서의 상태와 실행 증거
- `SCHOOLX_CODE_DESIGN.md` §6의 current implementation status

역사적 handoff의 당시 본문을 새 상태로 덮어쓰지 않는다.

이미 3,500줄을 넘은 `schoolx-code.spec.ts`에는 새 visual case를 추가하지 않는다.
기존 entry point와 800×500 regression만 focused 실행하고, 새 screenshot/assertion은
새 theme spec에 둔다.

## 9. 검증 runbook

먼저 repository Hermit 환경을 활성화한다.

```bash
. ./bin/activate-hermit
```

`build:e2e` 전에 `lsof -nP -iTCP:4173 -sTCP:LISTEN`으로 stale preview를 확인한다.
기존 server가 있으면 확인된 exact PID만 종료하고 fresh build를 사용한다.

Targeted gate:

```bash
pnpm --dir desktop exec biome check .
pnpm --dir desktop typecheck
pnpm --dir desktop exec node --import ./test-loader.mjs \
  --experimental-strip-types --test \
  src/shared/ui/schoolx-brand/schoolx-brand-assets.test.mjs
pnpm --dir desktop test
pnpm --dir desktop check:px-text
desktop/scripts/generate-schoolx-brand-assets.sh --check  # macOS only
pnpm --dir desktop build:e2e
pnpm --dir desktop exec playwright test \
  tests/e2e/schoolx-theme-screenshots.spec.ts \
  tests/e2e/boot-splash.spec.ts \
  tests/e2e/mobile-pairing-qr.spec.ts \
  tests/e2e/nostr-bind.spec.ts \
  --project=smoke
pnpm --dir desktop exec playwright test \
  tests/e2e/schoolx-code.spec.ts \
  --project=smoke --grep 'entry points|800x500'
```

File-size ratchet:

```bash
CHECK_FILE_SIZES_BASE=b1b283cd4c7f926e12eeee8ae1f38c7471922b16 \
  pnpm --dir desktop check:file-sizes
```

이 명령은 현재 known debt 때문에 **exit 1이 예상된다**. 이를 성공 gate처럼
`&&` 뒤에 연결하지 않는다. 현재 기준은 기존 19개 violation이다. 구현 시작과
종료에 같은 base로 출력한 `path / limit / actual lines` tuple을 비교한다. V1은
신규 path 0개, 기존 각 tuple의 line 증가 0개여야 하며 감소는 허용한다. 단순히
“19개”라는 개수만 같으면 기존 oversized 파일이 더 커져도 놓치므로 충분한
gate가 아니다. limit/allowlist를 완화하거나 990/999줄 파일에 로직을 쌓아서
통과시키지 않는다.

Screenshot과 최종 diff:

```bash
shasum -a 256 desktop/test-results/schoolx-theme/*.png
git diff --check
git status --short
```

E2E는 반드시 `pnpm build:e2e`를 사용한다. Plain `pnpm run build`는 mock Tauri
bridge를 제거하므로 UI failure처럼 보이는 거짓 결과를 만든다.

## 10. V1에서 의도적으로 남기는 residual

다음은 미적용 상태를 숨기지 않고 후속 visual slice로 남긴다.

- `LandingBees.tsx`의 bee swarm과 landing backdrop
- `OnboardingChrome.tsx`, `PendingInviteGate.tsx`, `SetupStep.tsx`,
  `BackupStep.tsx`의 Buzz mark/bee/fuzzy surface
- `MachineOnboardingFlow.tsx`의 `/landing/buzz-wordmark.png`
- `HostedCommunityOnboarding.tsx`, `NostrBindConsentDialog.tsx`의 onboarding shell과
  `components.css`의 chartreuse/light-blue, dotted/noisy gradient palette. V1은
  Nostr bind의 app icon만 바꾸며 그 surface re-theme은 후속이다.
- agent transcript/liveness의 FuzzyLogo surface
- horizontal SchoolX wordmark의 outlined lockup과 standalone reversed mark
- mobile app와 relay-served web client의 별도 visual rebrand
- 전 화면의 사용자 노출 `Buzz` 문자열/i18n 정리

Fizz/Honey/Bumble starter-team 이미지는 product logo가 아니라 persona content다.
별도 콘텐츠 결정 없이 바꾸지 않는다. Claude/Goose/Cursor 등 vendor logo와 사용자
avatar/community icon도 이번 scope 밖이다.

V1 완료 보고에는 반드시 다음 문장을 포함한다.

> Main app first-party theme와 core shell은 SchoolX V1을 사용한다. First-run
> onboarding과 일부 legacy branded status surface는 후속 slice로 남아 있다.

## 11. 보안·제품 비범위

이 visual slice는 다음을 열지 않는다.

- 새 Tauri command/event 또는 filesystem/process authority
- 새 route, network request, relay publish, media upload
- localStorage key rename 또는 community/relay theme sync
- SchoolX Code path/ref/OID/argv/Git authority 변경
- generic shell, Talk 자동 공유, transcript egress
- signing/notarization/release workflow 변경
- 직접 편집기나 SchoolX Code Phase 3 기능 추가

제품 동작이나 authority를 바꿔야만 디자인을 적용할 수 있다고 판단되면 V1을
확장하지 말고 별도 decision gate를 작성한다.

## 12. 현재 dirty worktree 보호

준비 시점에 다음 사용자 변경이 이미 존재했다. Visual 구현과 무관하므로 수정,
정리, stage, commit하지 않는다.

```text
 M .dockerignore
 M .gitignore
 M crates/buzz-core/src/relay.rs
 M deploy/compose/README.md
 M desktop/src-tauri/src/managed_agents/restore.rs
 M desktop/src-tauri/src/managed_agents/runtime.rs
 M desktop/src-tauri/src/managed_agents/runtime/tests.rs
 M desktop/src-tauri/src/managed_agents/runtime_commands.rs
?? deploy/compose/Dockerfile.local
?? supabase/
```

구현 commit을 요청받으면 exact visual paths만 stage하고 `git commit -s`를 사용한다.
사용자가 별도로 요청하지 않은 push나 release build는 하지 않는다.

## 13. 새 세션용 착수 요청

다음 문장을 새 세션에 그대로 전달할 수 있다.

```text
AGENTS.md와
docs/schoolx-2/SESSION_HANDOFF_20260825_SCHOOLX_VISUAL_SYSTEM_V1.md를 먼저
끝까지 읽고, 문서의 SchoolX first-party theme + core shell V1만 구현해줘.

내부 호환 ID/localStorage/data-buzz-*와 third-party theme 선택은 보존하고,
brand/를 canonical source로 사용해 semantic palette, Appearance preview/label,
stale cache prepaint, boot/community-switch mark, favicon, Nostr bind/Mobile QR app
icon을 완성해줘. Onboarding 전체 재설계와 제품 기능/authority 변경은 하지 마.

문서의 fault matrix, UI/E2E 완료 기준, file-size ratchet을 모두 검증하고,
기존 dirty worktree 파일은 stage하지 마. 구현 결과와 남은 onboarding residual을
정확히 구분해서 보고해줘.
```
