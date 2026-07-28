# SchoolX 2.0 제품 설정 계층 (세션 B)

이 문서는 SchoolX가 Buzz에서 갈라져 나오면서 갖는 **제품 정체성**을 고정한다.
세션 B의 완료 기준 중 "제품 문자열과 프로토콜 식별자를 구분한 설정 계층이
있다"의 산출물이며, 이후 세션은 여기 적힌 구분을 전제한다.

## 1. 세 종류의 문자열

코드에는 `buzz`처럼 보이는 문자열이 세 종류 있고, **제품 문자열만** 바꾼다.

| 종류 | 예 | 바꾸는가 | 이유 |
|---|---|---|---|
| 제품 문자열 | 딥링크 스킴, 번들 식별자, nest 디렉터리, 키체인 서비스명, 표시명 | **예** | 이 빌드를 가리키며, 하나라도 공유하면 Buzz와 데이터가 섞인다 |
| 프로토콜 식별자 | `buzz:nostr-identity`(audience), `buzz-nostr-identity`(protocol), Nostr kind, relay wire 값 | **아니오** | 릴레이·다른 클라이언트와 공유하는 어휘다. 한쪽만 바꾸면 interop이 깨진다 |
| 내부 네임스페이스 | localStorage 키 `buzz:text-scale`, DOM 이벤트 `buzz:open-create-agent` | **아니오** | 웹뷰 밖으로 안 나가고 localStorage는 이미 번들 식별자로 분리된다. 바꾸면 사용자 설정만 조용히 초기화된다 |

판별 기준: **릴레이나 다른 클라이언트가 알아채면 프로토콜, 이 기기만
알아채면 제품이다.**

## 2. 확정된 제품 정체성

| 항목 | 값 | 정의 위치 |
|---|---|---|
| 번들 표시명 | `SchoolX` | `desktop/src-tauri/tauri.conf.json` |
| UI 표시명 | 한국어 `스쿨엑스` / 영어 `SchoolX` | i18n `app.productName` |
| 번들 식별자 | `io.github.schoolx520.app` | `product::BUNDLE_IDENTIFIER` |
| dev 번들 식별자 | `io.github.schoolx520.app.dev` | `product::DEV_BUNDLE_IDENTIFIER` |
| 딥링크 스킴 | `schoolx` | `product::DEEP_LINK_SCHEME` |
| 에이전트 nest | `~/.schoolx` (dev `~/.schoolx-dev`) | `product::NEST_DIR_PROD` / `_DEV` |
| 키체인 서비스 | `schoolx-desktop` (dev `schoolx-desktop-dev`) | `product::KEYRING_SERVICE` / `_DEV` |

번들명이 ASCII인 것은 의도다. 앱 경로, DMG 볼륨, Windows/Linux 패키징이
예측 가능해야 하므로 파일시스템 이름은 영문으로 두고, 사용자에게 보이는
이름만 번역 문자열로 분리했다.

### 계층의 실제 위치

빌드가 셋이고 모듈 그래프를 공유하지 않으므로 같은 계층이 세 벌 있다.

| 파일 | 대상 |
|---|---|
| `desktop/src-tauri/src/product.rs` | 데스크톱 네이티브 |
| `desktop/src/shared/product/index.ts` | 데스크톱 프론트엔드 |
| `web/src/shared/lib/product.ts` | 릴레이가 서빙하는 웹 클라이언트 |

세 벌이 어긋나면 조용히 깨진다. 특히 웹 클라이언트는 데스크톱이 **등록하는**
스킴의 링크를 **생성**하므로, 웹만 안 바꾸면 초대·연결 버튼이 아무것도 열지
않는다. `product.rs`의 `tauri_config_matches_product_identity` 테스트가
`tauri.conf.json`과 Rust 상수의 일치를 강제한다.

## 3. 공유 자원 경계

번들 식별자만 바꾸면 격리되지 않는 자원이 있다. 이들은 `$HOME` 경로나 상수라
식별자와 무관하게 고정돼 있었다.

| 자원 | 원래 | 지금 | 안 바꿨다면 |
|---|---|---|---|
| 앱 데이터 디렉터리 | `xyz.block.buzz.app` | `io.github.schoolx520.app` | (식별자에서 자동 파생) |
| 에이전트 nest | `~/.buzz` | `~/.schoolx` | 두 제품이 같은 에이전트 지식(AGENTS.md, PLANS, WORK_LOGS)을 읽고 쓴다 |
| Huddle 모델 캐시 | `~/.buzz/models` | `~/.schoolx/models` | 위와 같음 |
| **OS 키체인 서비스** | `buzz-desktop` | `schoolx-desktop` | **두 제품이 같은 Nostr 신원과 에이전트 키를 읽고 덮어쓴다** |
| CLI 템플릿 경로 | `xyz.block.buzz.app/templates` | `io.github.schoolx520.app/templates` | `buzz channels create --template`이 Buzz의 템플릿을 읽는다 |

키체인은 계획서가 열거하지 않았던 항목이고, 보안상 가장 무겁다.
`secret_store.rs`에 "서비스명은 상수이며 번들 식별자를 따르지 않는다"고
명시돼 있었다.

### 손으로 훑어서는 다 못 찾는다

위 표는 계획서와 코드 읽기로 찾은 것이다. 이후 `just schoolx-upstream-check`를
만들어 기계적으로 훑자 **손으로 놓친 다섯 곳이 더 나왔다.** 전부 컴파일되고,
1,824개 Tauri 테스트를 통과하고, 타입 검사도 통과하던 코드다.

| 위치 | 증상 |
|---|---|
| `lib.rs` dev nest 판별 | `n == ".buzz-dev"` 비교가 `~/.schoolx-dev`와 영원히 불일치 → dev nest 마이그레이션이 아예 안 돎 |
| `managed_agents/storage.rs` | dev 키 마이그레이션이 `SecretStore::keyring("buzz-desktop")` — **Buzz의 프로덕션 키체인을 읽음**. 게이트도 옛 이름이라 우연히 도달 불가였을 뿐 |
| `commands/project_repo_paths.rs` | repos 루트 폴백이 `~/.buzz/REPOS` — 함께 설치된 Buzz의 체크아웃을 SchoolX 에이전트에게 넘김 |
| `managed_agents/managed_node_paths.rs` | 관리형 Node 툴체인과 npm prefix가 `<data-dir>/Buzz/` — 번들 식별자 디렉터리의 **형제**라 그 격리를 물려받지 않음 |
| `runtime/instance_reaper.rs` | 살아있는 데스크톱 프로세스 이름 목록이 `"Buzz"` → productName이 `SchoolX`가 되면 **모든 프로세스가 죽은 것으로 보여 reaper가 실행 중인 에이전트를 죽임** |

교훈은 두 가지다. 첫째, 제품 정체성은 한 곳에 모여 있지 않고 `$HOME` 경로,
데이터 디렉터리의 형제 폴더, 프로세스 이름 목록처럼 서로 다른 층에 흩어져
있다. 둘째, **이 종류의 실수는 타입 검사도 테스트도 잡지 못한다** — 이름이
안 맞으면 조건이 조용히 거짓이 될 뿐이다. 그래서 grep 기반 가드를 상시
검사로 남겼다.

의도적인 잔존 문자열(Cargo 패키지명, `buzz-agent` 런타임 표시명)은 소스에
`// schoolx:buzz-name-ok` 주석으로 명시적으로 빠져나간다. 정규식이 추측하게
두지 않고 코드를 읽는 사람이 볼 수 있는 자리에서 판단을 기록하기 위해서다.

### 남의 제품 데이터를 지우지도 않는다

격리는 읽기·쓰기만이 아니다. upstream의 초기화 경로는 `~/.sprout`(Buzz의
rename 이전 nest)을 삭제하고, `legacy_app_data_dir`로 이전 식별자의 앱
데이터를 복사한다. SchoolX에서는 둘 다 막았다.

- `migration::legacy_app_data_dir` → 항상 `None`. SchoolX는 선행 정체성이
  없다. Buzz의 것을 채택하면 첫 실행에 Buzz의 신원과 에이전트 설정을 복사한다.
- `migration::migrate_legacy_nest` → 항상 `false`. 디스크에 있는 `~/.sprout`,
  `~/.buzz`는 Buzz의 것이다.
- `reset.rs` 3단계 → `~/.sprout` 삭제를 제거. SchoolX 초기화가 함께 설치된
  Buzz의 지식을 파괴하면 안 된다.
- `scripts/reset-desktop-dev-state.sh` → `xyz.block.sprout.app.dev`,
  `sprout-desktop-dev`, `~/.sprout-dev` 삭제 패턴 제거.
- `scripts/instance-env.sh` → `xyz.block.sprout.app.dev/identity.key` 폴백
  제거. 그 키로 서명하면 SchoolX 이벤트가 Buzz 신원으로 나간다.

함수와 호출 지점 자체는 남겨 두었다. upstream이 부트 순서를 "레거시 채택은
모든 디스크 읽기보다 먼저"로 다루고 있어, seam을 유지하는 편이 이후 병합이
기계적이다.

## 4. 딥링크 정책

세 축을 따로 결정했다.

| 축 | 결정 |
|---|---|
| 새 링크 생성 | `schoolx://` 만 |
| OS 스킴 등록 | `schoolx` 만 |
| 과거 `buzz://` 수신 | OS 경유는 거부, 앱 내부 링크 텍스트는 읽음 |

**OS 등록을 하나로 둔 이유.** 두 앱이 같은 스킴을 등록하면 어느 쪽이 링크를
받을지 OS가 비결정적으로 정한다(macOS는 대개 마지막 등록 번들이지만
`lsregister` 재빌드를 넘어 보장되지 않는다). 등록을 `schoolx` 하나로 두면
Buzz를 같이 설치해도 라우팅이 결정적이다.

**앱 내부 레거시 링크를 읽는 이유.** rename 이전에 쓰인 메시지 — 그리고 같은
커뮤니티의 Buzz 사용자가 쓴 메시지 — 는 `buzz://message` 링크를 담고 있고,
그 메시지는 SchoolX 자신의 기록이다. 링크 텍스트를 파싱하는 것은 OS 라우팅이
아니라 이미 받아 온 텍스트를 다루는 일이라 격리와 무관하며, 거부하면 과거
메시지가 클릭 불가능한 100자 문자열로 썩는다. 그래서 **읽되 절대 생성하지
않는다.**

경계는 명확하다. OS 경계인 `deep_link.rs::handle_deep_link_url`은 `buzz://`를
사유와 함께 거부하고, 프론트엔드의 `messageLink.ts`는 링크 텍스트로서는
받아들인다.

### 5종 딥링크

`message`, `join`, `connect`, `add-community`, `nostr-bind` 모두 새 스킴에서
동작한다. `nostr-bind`의 `audience=buzz:nostr-identity`와
`protocol=buzz-nostr-identity`는 **프로토콜 값이라 그대로 둔다.**

### 고정된 테스트

| 대상 | 테스트 |
|---|---|
| 새 스킴이 5종 전부 허용 | `deep_link.rs::product_scheme_is_accepted_for_every_deep_link_kind` |
| 레거시 스킴이 5종 전부 거부(사유 포함) | `deep_link.rs::legacy_buzz_scheme_is_rejected_by_name` |
| 링크 생성은 레거시를 절대 안 씀 | `messageLink.test.mjs::buildMessageLink never emits the legacy scheme` |
| 레거시 링크 파싱·pill 렌더 | `messageLink.test.mjs` 3건 |
| 레거시 링크가 OS opener로 새지 않음 | `openPopoverLink.test.mjs::legacy-scheme message deep-link also routes in-app` |
| 레거시 링크 클릭 → 스레드 패널 (실제 앱) | `tests/e2e/navigation.spec.ts::legacy buzz:// message links still open the thread panel` |
| 두 스킴 정규식이 어긋나지 않음 | `messageLink.test.mjs::remarkMessageLinks pattern covers exactly the readable schemes` |

## 5. 마이그레이션 번호 예약 대역

SchoolX 전용 SQL 마이그레이션은 **`9001+`** 를 쓴다.

upstream 동기화에서 `0025_relay_invites.sql`이 SchoolX의
`0025_restrict_managed_agent_channel_add_policy.sql`과 번호가 겹쳤다. sqlx는
`_sqlx_migrations`를 version으로 키잉하면서도 중복 버전을 컴파일 타임에
거부하지 않아, 빌드는 통과하고 개발 DB는 둘 중 하나를 영구히 적용하지 않는
상태가 됐다. `0026`으로 올리면 다음 upstream 마이그레이션과 또 겹치므로 대역
자체를 분리했다. 정렬상 항상 upstream 뒤에 오는데, SchoolX 마이그레이션이
upstream이 만든 컬럼을 읽으므로 이 순서가 필요하다.

`buzz-db::migration::tests::embedded_migrator_contains_consolidated_initial_schema`가
개수와 대역 침범을 함께 단언한다.

## 6. 아직 결정되지 않은 것

정직하게 남긴다. 세션 B는 이 항목들을 **완료로 표시하지 않는다.**

**아이콘과 색상.** `desktop/src-tauri/icons/`는 여전히 Buzz 아이콘이다.
`tauri.conf.json`의 `bundle.icon`과 DMG 배경도 그대로다. 실제 이미지 자산이
필요하며 디자인 결정이다. 앱은 동작하지만 배포본은 Buzz 아이콘을 달고 나온다.

**업데이트 서버.** `plugins.updater.endpoints`는 upstream에서도 `[]`이며
SchoolX도 비워 두었다. 자동 업데이트를 붙이려면 endpoint와 서명 공개키를
정해야 한다.

**배포 서명 주체.** Buzz의 서명 빌드는 `squareup/sprout-releases`(Block 내부
Buildkite)가 만든다. SchoolX는 그 파이프라인을 쓸 수 없다. macOS 배포에는
별도의 Apple Developer 계정과 notarization이 필요하다.

**Apache-2.0 §4(b) 고지.** `BASELINE.md`가 기록한 대로, 수정 사실을 배포본에
명시할 의무가 남아 있다. Phase 6(패키징)의 오픈소스 고지 화면에서 처리한다.

**화면별 문자열.** `app.productName` 키만 만들었다. 설정 패널 등에 남은 영어
"Buzz" 문구는 세션 C(핵심 화면 번역) 범위다.

## 7. 관련 문서

- [`DEVELOPMENT_PLAN.md`](DEVELOPMENT_PLAN.md) — §4.2 적용 원칙, §7 Phase 2
- [`IMPLEMENTATION_HANDOFF.md`](IMPLEMENTATION_HANDOFF.md) — 세션 순서와 유지할 사실
- [`SECURITY_CONTRACT.md`](SECURITY_CONTRACT.md) — 세션 A 접근 계약
- [`BASELINE.md`](BASELINE.md) — 실행 환경과 검증 명령
