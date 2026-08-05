# 세션 핸드오프 (2026-08-05)

새 세션이 여기부터 이어간다. 프로젝트 전반의 상시 문서는
[`IMPLEMENTATION_HANDOFF.md`](IMPLEMENTATION_HANDOFF.md)이고, 이 문서는
**이번 이틀(8/4–8/5)에 무엇이 끝났고 무엇이 열려 있는지**만 다룬다.

## 0. 첫 5분에 할 것

```bash
cd /Users/kim-yonghun/Development/schoolX_v2.0   # 워크트리 아님 — §5 참조
. ./bin/activate-hermit
git status -sb          # 깨끗해야 한다
git log --oneline -3
```

전부 푸시되어 있다. **다음 작업은 §2.2**(온보딩 사람 검증) — §2.1은 닫혔다.

## 1. 이번에 끝난 것

| 세션 | 내용 | 커밋 |
|---|---|---|
| D2 | **Phase 3 닫기** — 완료 기준 7/7 충족 | `ad9b9b2c`–`a8dff5ce` |
| D3 | **catalog 재생성** — `deleted`·`not_owned`에 답할 수단 | `70bdc977`–`1db46d8c` |
| — | **실제 앱에서 catalog 적용 확인** (Postgres 직접 조회) | `953cc95b` |
| G | **자체 호스팅 릴레이 온보딩** — 학교가 자기 릴레이로 들어오는 문 | `13a93ca5`–`fae8cc93` |
| — | **제품명 SchoolX 전환** (사용자 노출 96줄/56파일) | `0de3d696` |
| — | **팀 배포용 미서명 빌드 워크플로** | `c935b318` |

각 세션의 「넘긴 것」과 이후 세션이 전제해야 할 사실은
`IMPLEMENTATION_HANDOFF.md`의 해당 절에 있다. 여기서 반복하지 않는다.

## 2. 지금 열려 있는 것 — 우선순위 순

### 2.1 팀 빌드 워크플로 — **첫 실행 성공 (2026-08-05)**

`c935b318`이 만든 `.github/workflows/schoolx-team-build.yml`이
[run 30996818994](https://github.com/DevYonghunT/buzz/actions/runs/30996818994)에서
처음 돌았고 두 잡 모두 통과했다. 절차와 팀원 배포 안내는
[`TEAM_BUILD.md`](TEAM_BUILD.md).

| 아티팩트 | 크기 | 잡 시간 |
|---|---|---|
| `schoolx-macos-0.5.3-team.1` (DMG) | 67MB | 14분 |
| `schoolx-windows-0.5.3-team.1` (NSIS) | 49MB | 29분 |

돌리기까지 두 가지가 막고 있었다.

**(a) macOS 사이드카 경로 — 확정적 실패였다** (`93da132a`에서 고침).
`bundle-sidecars.sh`는 `$1`이 있으면 「cargo를 `--target`으로 불렀다」로 읽고
`target/<triple>/release`를 본다. macOS 잡은 `--target` 없이 빌드하면서 호스트
트리플을 넘기고 있어서 빈 디렉터리를 뒤지고 죽을 자리였다. 저장소의 나머지 세
잡이 이미 이 짝을 지킨다 — `release.yml` arm64는 인자 없이, Intel·Windows와
`windows-canary.yml`은 `--target`과 그 트리플을 함께 넘긴다.

**(b) `workflow_dispatch`는 기본 브랜치에 있는 파일에만 반응한다.** 포크의
기본 브랜치가 upstream 미러인 `main`이라 워크플로가 목록에 **뜨지도 않았다**.
기본 브랜치를 `codex/schoolx-2-foundation`으로 바꿔 풀었다 — SchoolX 커밋
91개가 전부 그쪽에 있으므로 실제 작업 위치와도 맞는다. 파급은 없음을
확인했다: `ci.yml`은 `branches: [main, release]` 명시 목록이라 작업 브랜치
push에 새로 돌지 않고, canary·sprig 계열은 `default_branch` 조회가 아니라
`refs/heads/main` 문자열 비교이며, pre-push의 `check-branch-skew.sh`도
`origin/main`을 하드코딩한다.

**Intel Mac 대응을 이어서 더했고 검증했다** (`e7efde2d`). 첫 실행이 낸 DMG는
`macos-latest`(Apple Silicon) 호스트 빌드라 arm64 전용이었고, 팀에 Intel Mac이
있는 것이 확인돼 `macos-intel` 잡을 더했다 — `--target x86_64-apple-darwin`
교차 컴파일로, `release.yml`의 Intel 잡과 같은 짝이다.
[run 30999329683](https://github.com/DevYonghunT/buzz/actions/runs/30999329683)에서
세 잡 모두 통과했다: `schoolx-macos-apple-silicon-0.5.3-team.2` 67MB,
`schoolx-macos-intel-0.5.3-team.2` 70MB, `schoolx-windows-0.5.3-team.2` 49MB.

**초록불만 보지 말고 아키텍처를 확인했다.** Intel 잡 로그의 번들 경로가
`target/x86_64-apple-darwin/release/bundle/dmg/`이고 tauri가 파일명을
`SchoolX_0.5.3-team.2_x64.dmg`로 붙였다 — 교차 컴파일이 실제로 먹었다는 뜻이다.
잡이 통과했다는 사실만으로는 arm64를 두 번 만들지 않았다고 말할 수 없다.

아키텍처를 잘못 받으면 앱이 실행되지 않는데 그 증상이 미서명 경고와
구별되지 않는다. 고르는 법을 `TEAM_BUILD.md` §2 맨 앞에 적어 뒀다.

### 2.2 온보딩 사람 검증 ← **다음 작업 (팀 배포 대기 중)**

세션 G의 완료 기준은 「소스를 보지 않은 사람이 화면 안내만으로 들어간다」인데
**아직 아무도 밟아 보지 않았다.** Playwright가 두 진입점의 존재와 스크린샷을
냈지만 그건 존재 확인이지 사용성 확인이 아니다.

막힌 이유: 이 기기의 앱이 이미 커뮤니티에 붙어 있어 `community.needsSetup`이
거짓이라 온보딩 화면에 닿지 못한다. `just dev`는 초기화 스크립트를 부르지
않는다 — `BUZZ_RESET_WEBVIEW_STATE=1`을 실제로 쓰는 것은
`desktop-standalone`뿐이다.

```bash
just relay                        # 터미널 1
just fresh=1 desktop-standalone   # 터미널 2 — 인스턴스 상태 초기화 후 실행
```

`fresh=1`은 `~/Library/{Application Support,Caches,WebKit,HTTPStorages,…}/io.github.schoolx520.app.dev`와
**dev 키체인 신원**을 지운다. `just reset`은 쓰지 말 것 — 그건 Postgres·MinIO
볼륨까지 지워 8/4에 만든 catalog 방과 검증 증거가 날아간다.

**이 검증은 팀 빌드를 받은 사람이 하는 게 더 낫다.** 이미 화면을 다 본 사람이
하면 「소스를 보지 않은 사람」 조건이 성립하지 않는다. §2.1이 닫히면서 그 길이
열렸다 — 나눠줄 파일은
[run 30999329683](https://github.com/DevYonghunT/buzz/actions/runs/30999329683)의
셋이다.

| 아티팩트 | 받을 사람 |
|---|---|
| `schoolx-macos-apple-silicon-0.5.3-team.2` | M1 이후 Mac |
| `schoolx-macos-intel-0.5.3-team.2` | Intel Mac |
| `schoolx-windows-0.5.3-team.2` | Windows |

전달할 때 `TEAM_BUILD.md` §2를 같이 보낸다(파일 고르는 법 + 두 OS의 미서명
경고 우회). 아티팩트 다운로드에는 GitHub 로그인이 필요하므로, 직접 받아
파일로 건네는 쪽이 막힘이 없다.

**돌아온 피드백이 §2.3의 범위를 정한다.** 어느 문구에서 막혔는지 알면 첫
동선 162개 파일을 다 건드리지 않아도 된다 — 그래서 번역보다 이쪽이 먼저다.

### 2.3 한국어 번역 (B 작업)

제품명 전환(A)은 끝났고 **번역은 시작도 안 했다.** 규모 (2026-08-05 실측):

- i18n 키가 **85개뿐**인데 화면은 수십 개다 → 대부분의 화면이 애초에 i18n을
  타지 않고 소스에 영어가 박혀 있다. `en`·`ko` 둘 다 85개로 패리티는 맞다.
- 네임스페이스는 5개: `app`, `settings`, `time`, `appearance`, `catalog`.
- 첫 동선 feature의 `.tsx` 수와 그중 `t()`를 쓰는 파일 수:

  | feature | `.tsx` | `t()` 쓰는 파일 |
  |---|---|---|
  | onboarding | 33 | **0** |
  | sidebar | 15 | **0** |
  | channels | 37 | **0** |
  | messages | 44 | 3 |
  | settings | 33 | 3 |

  162개 중 6개뿐이고, **팀원이 가장 먼저 보는 온보딩이 0개다.** 착수하면
  온보딩부터가 맞다 — 단, 범위는 §2.2 피드백을 받고 정한다.

전부 하지 말고 **팀원이 실제로 밟는 첫 동선**(온보딩 → 사이드바 → 채널/메시지
→ 설정 진입부)만 키로 옮기는 범위로 이미 합의했다. 나머지(에이전트·워크플로·
git·huddle 설정)는 피드백 대상이 아니다.

**주의**: 새 i18n 네임스페이스를 만들면 `en`·`ko`·`APP_I18N_NAMESPACES`를 한
번에 바꿔야 하고, 빠뜨리면 fallback이 구제하지 못해 한국어에 원시 키가
노출된다(세션 C 사실 1번). 기존 네임스페이스에 넣으면 그 위험이 없다.

### 2.4 나머지 8개 업무방 콘텐츠

낮은 추론. 스키마와 적용 API는 고정됐다. 사용자가 「나중에 천천히」로
분류했다.

### 2.5 아이콘·로고 자산

디자인 결정 대기. 지금도 Buzz 아이콘이다.

## 3. 이번에 알게 된 것 중 다음 세션이 밟기 쉬운 함정

### 3.0 팀 빌드에 `TAURI_BUNDLER_DMG_IGNORE_CI`는 필요 없다 — 고치지 말 것

`release.yml`(둘)과 `signed-macos-canary.yml`의 macOS 잡은 모두 이 변수를
켜므로, `schoolx-team-build.yml`에 없는 것이 누락으로 보인다. **아니다.**
tauri는 이 변수가 없으면 CI에서 `bundle_dmg.sh`에 `--skip-jenkins`를 붙일
뿐이고 **DMG는 그대로 만든다** — 실제 실행 로그에서 `--skip-jenkins`가 붙은
채 DMG가 생성된 것을 확인했다. 그 셋이 켜는 이유는 직후에
`set-dmg-finder-text-size.sh`로 Finder 창 외형을 손보기 때문이고, 팀 빌드는
그 단계가 없다.

### 3.1 `.env`에 멤버십을 켜 두면 E2E가 통째로 깨진다

`just`는 `set dotenv-load := true`라 `.env`를 **모든 레시피**에 먹인다.
`BUZZ_REQUIRE_RELAY_MEMBERSHIP=true`를 상시로 두면 `just test-e2e`가 띄우는
릴레이도 멤버십을 강제하고, 매번 새 키로 채널을 만드는 스위트가 전부
`relay_membership_required`로 거부된다 — **0.24초 만에 전멸하고, 증상만으로는
회귀와 구별되지 않는다.**

현재 `.env`에는 세 줄이 **주석 처리**되어 있고 사용법이 그 자리에 적혀 있다.
앱에서 catalog 적용을 검증할 때만 그 실행에 얹는다.

**즉시 전멸하는 테스트 실패는 제품보다 환경을 먼저 의심한다.**

### 3.2 멤버십 관련 환경변수는 셋이 한 묶음이다

`BUZZ_REQUIRE_RELAY_MEMBERSHIP` + `RELAY_OWNER_PUBKEY` +
`BUZZ_RELAY_PRIVATE_KEY`. 하나라도 빠지면 릴레이가 **기동을 거부하고**, 각
오류는 자기 변수만 지목한다 — 미리 셋을 다 채우지 않으면 한 번에 하나씩
만난다(8/4에 실제로 두 번 튕겼다). 절차는 `CONTRIBUTING.md`의 「Running with
relay membership enforced」.

### 3.3 `schoolx:buzz-name-ok` 마커는 같은 줄에 있어야 한다

제품 식별자 검사(`just schoolx-upstream-check`)는 줄 단위 grep이라 **앞줄
주석은 걸리지 않는다.** 의도적 유지는 그 줄 끝에 붙인다.

`just schoolx-upstream-check all`(전체 트리)이 **이제 통과한다.** BASELINE이
「전체 트리 스캔은 이 범위 밖에서 실패한다」로 남겨둔 조건이 닫혔으므로,
앞으로 이 검사가 깨지면 그건 새로 들어온 것이다.

### 3.4 기계적 치환은 주석과 테스트 기대값을 건드린다

제품명 전환에서 세 번 틀렸다 — kind 9005 주석이 "SchoolX-native"가 되고
(그 kind는 Buzz가 정의했다), `BuzzTheme` 주석이 바뀌고, 테스트 파일에서
과하게 치환해 실패가 늘었다. **셋 다 게이트가 잡았다.** 치환 후에는
`git diff`에서 주석 안 변경을 따로 훑는다.

### 3.5 다이얼로그 위에 무언가를 얹으려면 그 다이얼로그의 자식이어야 한다

세션 G에서 탈출구 링크를 다이얼로그 **밖**에 `fixed`로 뒀더니 오버레이 블러에
묻히고 포인터도 가로채였다. `toBeVisible()`은 통과하고 `click()`이 타임아웃
나는 모양이라 **존재 단언만으로는 잡히지 않는다.**

### 3.6 e2e 하네스는 영어 로케일로 렌더한다

Playwright 스펙에서 한국어 문구를 단언하면 실패한다. 문구 대신 `data-testid`로
단언한다 — 번역을 고칠 때마다 테스트가 깨지는 것도 막는다.

### 3.7 `just ci`는 한 명령으로 완주하지 못하고, 묶는 개수도 한도에 포함된다

에이전트 하네스의 10분 제한 때문이다. 구성 레시피를 하나씩 포그라운드로
돌린다. 8/5에는 여섯 개를 한 셸 루프로 묶었다가 `mobile-test` 도중 걸렸다.

## 4. 상태 요약

```
브랜치: codex/schoolx-2-foundation  ← 이제 저장소 기본 브랜치이기도 하다
원격:   origin/codex/schoolx-2-foundation — 로컬과 동기
워킹트리: 깨끗
```

**Phase 3 완료.** 남은 Phase는 `DEVELOPMENT_PLAN.md` 참조.

마지막 전체 게이트(8/5, 세션 G): 구성 레시피 14개 exit 0,
`schoolx-upstream-check` 3/3, `e2e_workspace_catalog` 5/5,
`e2e_access_matrix` 17/17, Playwright 7개. **단, 제품명 전환(`0de3d696`)
이후로는 전체 게이트를 돌리지 않았다** — 데스크톱 단위 테스트 3,929개와
typecheck·biome·제품 식별자 검사는 통과했으나 Rust·mobile·E2E는 그 커밋
이후 미실행이다. 푸시 전 pre-push 훅이 대부분을 덮는다.

## 5. 작업 환경 주의

- **메인 체크아웃에서 작업한다** (`/Users/kim-yonghun/Development/schoolX_v2.0`).
  워크트리에서는 `just desktop-tauri-fmt`가 실패해 pre-commit이 막힌다.
- `git commit -s` 필수 (DCO). `just hooks`가 commit-msg 훅을 설치한다.
- **머신 부하를 먼저 본다.** 8/5 측정 중 프로세스 1,251개에 부하 평균 315였고
  `desktop-test`가 평소 2–5분에서 10분을 넘겼다. 그 상태에서 잰 시간은
  `BASELINE.md`에 넣지 않는다 — 세션 D2·D3 표와 나란히 두면 회귀로 읽힌다.
- **10분 한도**: 오래 걸리는 명령은 백그라운드로 돌리고 완료를 확인한다.
  푸시도 pre-push 훅 때문에 10분을 넘긴다.
