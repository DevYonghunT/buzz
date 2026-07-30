# SchoolX 2.0 재현 가능한 기준선

이 문서는 “원본 Buzz에서 이미 실패하던 항목”과 “SchoolX 변경으로 생긴
실패”를 구분하기 위한 실행 기록이다. 결과가 없는 항목을 통과로 추정하지
않으며 `미검증`으로 남긴다.

## snapshot

| 구분 | 값 |
|---|---|
| 원본 비교 snapshot | `acfbb1bb6af54cb29cb152496ff43b8285dcb8cf` |
| i18n foundation snapshot | `a3e1ca4fb5f199b8ade62e14c120967ddab190d9` |
| 측정 대상 snapshot | `cbe1e4b9` (측정 시작 시점 `7d312389`) |
| 개발 브랜치 | `codex/schoolx-2-foundation` |
| origin | `https://github.com/DevYonghunT/buzz.git` |
| upstream | `https://github.com/block/buzz.git` |
| 검토 환경 | macOS arm64 (Darwin 25.5.0), Asia/Seoul |
| 기록일 | 2026-07-25 |

원본 비교 snapshot은 foundation commit의 parent다. upstream 최신 상태와
같다는 의미는 아니므로 실행 전 `git fetch upstream` 후 관계를 별도로
기록한다.

desktop 명령 6종은 `7d312389`에서 측정했고, 이후 `cbe1e4b9`는
`crates/buzz-db/src/migration.rs`의 카운트 단언 한 줄만 바꾼 커밋이라
desktop 결과에 영향을 주지 않는다. `just test` 결과는 `cbe1e4b9` 기준이다.

## 도구 버전

2026-07-25에 repo Hermit 환경에서 확인한 값이다.

| 도구 | 버전 |
|---|---|
| Node.js | `v24.14.0` |
| pnpm | `11.4.0` |
| rustc | `1.95.0 (59807616e 2026-04-14)` |
| cargo | `1.95.0 (f2d3ce0bd 2026-03-21)` |
| just | `1.46.0` |
| colima | `0.10.3` |
| Docker Engine | `29.5.2` (colima VM, linux/aarch64, 4 CPU / 6GiB) |

Docker Desktop은 설치하지 않았다. 이 환경의 Docker 데몬은 colima가
제공한다. `just test`가 요구하는 Postgres·Redis는 이 데몬 위에서 돈다.

모든 실행은 먼저 다음 명령으로 repo toolchain을 활성화한다.

```bash
. ./bin/activate-hermit
```

## 실행 증거

두 snapshot에서 같은 명령을 같은 toolchain으로 실행한 결과다. 시각은 UTC다.

| snapshot | command | exit | passed | failed | 소요 | 분류 |
|---|---|---:|---:|---:|---:|---|
| `acfbb1bb` | `git status --short` | 0 | - | - | 2s | - |
| `acfbb1bb` | `pnpm --dir desktop typecheck` | 0 | - | - | 41s | 통과 |
| `acfbb1bb` | `pnpm --dir desktop check` | 0 | - | - | 13s | 통과 |
| `acfbb1bb` | `pnpm --dir desktop test` | 0 | 3,401 | 0 | 83s | 통과 |
| `acfbb1bb` | `pnpm --dir desktop build` | 0 | - | - | 54s | 통과 |
| `acfbb1bb` | `just desktop-tauri-test` | 0 | 1,560 | 0 | 475s | 통과 (13 ignored) |
| `7d312389` | `git status --short` | 0 | - | - | 1s | - |
| `7d312389` | `pnpm --dir desktop typecheck` | 0 | - | - | 44s | 통과 |
| `7d312389` | `pnpm --dir desktop check` | 0 | - | - | 12s | 통과 |
| `7d312389` | `pnpm --dir desktop test` | 0 | 3,420 | 0 | 106s | 통과 |
| `7d312389` | `pnpm --dir desktop build` | 0 | - | - | 106s | 통과 |
| `7d312389` | `just desktop-tauri-test` | 0 | 1,560 | 0 | 649s | 통과 (13 ignored) |
| `cbe1e4b9` | `just test` | 1 | - | 2 | 32s | 아래 참조 |

desktop 단위 테스트 수 차이(3,401 → 3,420)는 SchoolX가 추가한 19개
테스트다. 양쪽 모두 실패 0이다.

### 세션 B (2026-07-28, upstream `925a9a7b` 병합 후)

병합으로 upstream 테스트가 늘어 부모 스냅샷과 개수를 직접 비교할 수 없다.
기준은 "이 트리에서 모두 통과"다.

| command | exit | passed | failed | 분류 |
|---|---:|---:|---:|---|
| `pnpm --dir desktop typecheck` | 0 | - | - | 통과 |
| `pnpm --dir desktop check` | 0 | - | - | 통과 |
| `pnpm --dir desktop test` | 0 | 3,745 | 0 | 통과 |
| `pnpm --dir desktop build` | 0 | - | - | 통과 |
| `just desktop-tauri-test` | 0 | 1,824 | 0 | 통과 (14 ignored) |
| `pnpm --dir web check` / `typecheck` | 0 | - | - | 통과 |
| `cargo test -p buzz-cli --lib` | 0 | 251 | 0 | 통과 |
| `just desktop-tauri-clippy` | 0 | - | - | 통과 |
| playwright `navigation`·`i18n`·`invite-link-copy` (smoke) | 0 | 18 | 0 | 통과 (1 skipped) |
| `just test-e2e e2e_access_matrix` | 0 | **17** | 0 | 통과 — 세션 A 계약이 병합 후에도 성립 |
| `just test` | 1 | - | 2 | `fake_llm.rs` steer 계열, 기존 실패 (아래) |

소요 시간은 cargo/vite 캐시 상태에 좌우된다. 위 값은 원본 worktree가
cold, 현재 checkout이 warm인 상태에서 측정했으므로 성능 비교에 쓰지
않는다.

### 세션 D (2026-07-30, 워크스페이스 catalog 최종 게이트)

측정 대상은 `3b0859d6`이고 실행 직전 `git status --short`는 비어 있었다.
**이후 브랜치 리뷰 수정으로 HEAD가 이 커밋을 여러 번 앞질렀다 — 아래 표는
`3b0859d6` 시점의 기록 그대로 두고, 현재 HEAD와의 관계는 「브랜치 리뷰 수정
이후 HEAD 재지정」 절에 별도로 적는다.**
시각은 UTC이며 cargo 캐시는 첫 명령에서 cold, 이후 warm이다.

| command | 시작(UTC) | exit | passed | failed | 소요 | 분류 |
|---|---|---:|---:|---:|---:|---|
| `just ci` | 17:21:13 | **143** | - | - | 10m(중단) | infra — 아래 참조 |
| `just fmt-check clippy` | 17:32:12 | 0 | - | - | 9s | 통과 |
| `just desktop-check desktop-tauri-fmt-check` | 17:32:28 | 0 | - | - | 20s | 통과 |
| `just desktop-tauri-clippy` | 17:32:54 | 0 | - | - | 28s | 통과 |
| `just web-check mobile-check` | 17:33:28 | 0 | - | - | 23s | 통과 |
| `just test-unit` | 17:33:58 | 0 | 6 suites | 0 | 13s | 통과 — `schoolx-catalog` 66/0 |
| `just desktop-test` | 17:34:27 | 0 | 3,756 | 0 | 95s | 통과 (58 suites, skipped 0) |
| `just desktop-build` | 17:36:08 | 0 | - | - | 49s | 통과 (`tsc && vite build`) |
| `just desktop-tauri-check` | 17:37:05 | 0 | - | - | 60s | 통과 |
| `just desktop-tauri-test` | 17:38:14 | 0 | 1,834 | 0 | 143s | 통과 (14 ignored) |
| `just web-build mobile-test` | 17:40:46 | 0 | 824 | 0 | 147s | 통과 (mobile 1 skipped) |
| `just test-e2e e2e_access_matrix` | 17:44:09 | 0 | **17** | 0 | 101s | 통과 — 세션 A 계약 유지 |
| `just test-e2e e2e_workspace_catalog` | 17:46:00 | 0 | **4** | 0 | 32s | 통과 |
| `just schoolx-upstream-check` | 17:46:38 | 0 | 3/3 | 0 | 1s | 통과 |

로그는 세션 로컬 scratchpad에 있었고 repo에 보존하지 않았다. 위 명령을 그대로
재실행하면 같은 값을 얻는다.

#### `just ci`가 한 명령으로는 완주하지 못한다

이 세션의 도구 실행 한도가 명령 하나당 10분이다. `just ci`는 cold 캐시에서
`cargo clippy --workspace --all-targets` 한 단계에만 **9분 44초**를 쓰므로
한 번의 호출로 끝나지 않는다 (exit 143 = 하네스가 SIGTERM). 제품 실패가
아니다.

`just ci`는 정의상 `check test-unit desktop-test desktop-build
desktop-tauri-check desktop-tauri-test web-build mobile-test`이고,
`check`는 다시 `fmt-check clippy desktop-check desktop-tauri-fmt-check
desktop-tauri-clippy web-check mobile-check`다. 위 표의 2–11행이 이 목록
전체를 원래 순서대로 나눠 돌린 것이며 전부 exit 0이다. 캐시가 warm이면
합계는 약 587초로, 10분 한도에 아슬아슬하게 걸린다.

**CI에서는 이 분해가 필요 없다.** 사람이나 에이전트가 이 환경에서 돌릴 때만
해당한다.

#### `just schoolx-upstream-check`의 검사 3이 새 크레이트를 훑지 않는다

검사 3(제품 식별자)의 대상 디렉터리는 `desktop/src-tauri/src`, `desktop/src`,
`crates/buzz-cli/src`, `web/src` 넷으로 고정돼 있다(`justfile`의 `roots`).
세션 D가 추가한 `crates/schoolx-catalog/src`와
`crates/buzz-test-client/tests/`는 여기에 들지 않는다. 이번 실행이 훑은 29개
파일에 세션 D의 데스크톱·CLI 파일은 모두 들어 있었고, 범위 밖 경로는 같은
grep을 손으로 돌려 확인했다 — 식별자 리터럴 없음.

**앞으로 SchoolX가 저 넷 밖에 크레이트를 추가하면 검사 3이 그것을 보지
못한다.** `roots` 확장 여부는 세션 소유자가 결정할 사항이라 이번에는 고치지
않았다.

#### 브랜치 리뷰 수정 이후 HEAD 재지정 (2026-07-31)

위 표(측정 대상 `3b0859d6`)는 이후의 브랜치 전체 리뷰 수정을 반영하지 않는다.
`3b0859d6` 위로 `68349244`(구현 결과 기록), `ce12b5cb`(비멤버 차단 테스트에
positive control 추가), `042e59db`(provenance를 도출된 채널에 묶는 결합
검사 추가 — `WORKSPACE_CATALOG.md` §11의 「브랜치 리뷰 수정」이 그 결과를
기록한다)가 얹혔고, 그 위에 이 문서를 포함해 §4·§6 정정, `roots` 확장(바로
위 절), owner 오류 지역화를 담은 커밋이 지금의 HEAD다
(`git log -1 --format='%H %s'`로 정확한 값을 확인한다).

이 마지막 커밋에서 실제로 재실행해 확인한 것은 아래뿐이다. 위 표의 나머지
값(`just ci` 분해 실행, `desktop-tauri-test`, `e2e_workspace_catalog` 등)은
**재실행하지 않았다** — 여전히 `3b0859d6`(일부는 `042e59db`, §11 참고) 시점의
기록이므로 그 시점의 증거로만 읽는다.

| command | 결과 |
|---|---|
| `cargo test -p schoolx-catalog` | 71 passed / 0 failed |
| `cargo fmt -p schoolx-catalog` | 이미 규칙을 만족함 — 추가 변경 없음 |
| `cargo clippy -p schoolx-catalog --all-targets -- -D warnings` | 경고 0 |
| `pnpm --dir desktop typecheck` | 통과 |
| `pnpm --dir desktop test` | 3,756 passed / 0 failed |
| `just schoolx-upstream-check` (기본 범위 — 마지막 sync 이후 변경분) | 3/3 통과, 대상 37개 파일 |

바로 위 절이 적은 "검사 3이 새 크레이트를 훑지 않는다"는 이 커밋으로
**닫혔다.** `justfile`의 `roots`에 `crates/schoolx-catalog/src`를 추가했고,
위 `schoolx-upstream-check` 실행이 그 경로의 8개 파일(`catalog.rs`·
`channel_id.rs`·`effects.rs`·`ledger.rs`·`lib.rs`·`preflight.rs`·
`provenance.rs`·`saga.rs`)을 대상에 포함해 통과시켰음을 확인한다.
`crates/buzz-test-client/tests/`는 여전히 범위 밖이다 — 이번 수정이 다루는
문제가 아니라 손대지 않았다.

**참고 — 전체 트리 스캔(`since=all`)은 이 범위 밖에서 실패한다.** 확인
차원에서 `just schoolx-upstream-check all`도 돌려 봤는데, 아래 파일에서
히트가 났다.

- `desktop/src-tauri/src/managed_agents/repos.rs` (테스트 내 `.buzz`
  디렉터리명 다수)
- `desktop/src-tauri/src/migration/materialize.rs`
- `desktop/src/features/agents/ui/agentSessionToolClassifier.ts`
- `desktop/src/features/profile/ui/NostrBindConsentDialog.tsx`
- `web/src/features/repos/mock-repos.ts`
- `web/src/features/repos/ui/ReposPage.tsx`

전부 `roots`에 이번에 추가한 `crates/schoolx-catalog/src`가 **아니라**
원래부터 있던 네 디렉터리 안이고, 이번 세션이 만들거나 건드린 파일이 아니다
— 검사 3의 기본 호출(마지막 sync 이후 변경분)은 그 파일들이 바뀌지 않아 스캔
대상에서 빠지므로 이 실패를 가리지 않는다. 실제 리터럴인지 정규식 오탐(예:
파일 바깥 이름이 아닌 `#[cfg(test)] mod tests`용 임시 디렉터리)인지는
확인하지 않았다 — 이번 다섯 항목의 범위 밖이라 별도 세션의 판단에 맡긴다.

### 검증되지 않은 첫 실행과 그 원인

첫 실행에서 `cargo test --manifest-path desktop/src-tauri/Cargo.toml`이
양쪽 snapshot 모두 exit 101로 실패했다. 둘 다 제품 실패가 아니었다.

- **명령 자체가 틀렸다.** 이 명령은 `_ensure-sidecar-stubs`를 건너뛰므로
  Tauri가 compile time에 `binaries/buzz-acp-<target>` externalBin을
  찾지 못해 build.rs에서 panic한다. 깨끗한 checkout에서는 통과할 수 없다.
  올바른 명령은 `just desktop-tauri-test`이며 필수 명령 목록을 고쳤다.
- **원본 worktree는 네트워크가 필요했다.** `sherpa-onnx-sys` build script가
  GitHub release에서 static lib을 받으려다 DNS 실패로 죽었다. 아래 재실행
  절차의 `SHERPA_ONNX_ARCHIVE_DIR`로 해결한다.

### `just test` 실패 분류

`cbe1e4b9`에서 crate 7개가 통과하고 workspace 통합만 실패했다.

| 통과 | buzz-core, buzz-auth 단위, buzz-db 단위, buzz-db 통합, buzz-conformance, buzz-push-gateway, buzz-auth 통합 |
|---|---|
| 실패 | workspace 통합 (`buzz-agent`) |

**SchoolX 회귀 1건 — 수정 완료.**
`buzz-db::migration::tests::embedded_migrator_contains_consolidated_initial_schema`가
`left: 25 / right: 24`로 실패했다. 이 테스트는 embedded migrator의 개수를
하드코딩으로 단언해 마이그레이션 추가가 검토 없이 startup state에 들어가는
것을 막는 가드다. `0025_restrict_managed_agent_channel_add_policy.sql`을
추가하면서 가드를 함께 갱신하지 않은 것이 원인이며 `cbe1e4b9`에서 25로
고쳤다. 마이그레이션에 데이터 `UPDATE`를 두는 것 자체는 0022·0024의 기존
패턴을 따른 것이다.

**기존 실패 — `buzz-agent` 테스트의 환경 플래키.**
`crates/buzz-agent/tests/`의 비동기 테스트가 실행마다 다른 이름으로
실패한다. SchoolX는 이 crate를 수정하지 않았다. 양쪽 snapshot에서
`cargo test -p buzz-agent`를 3회씩 실행한 결과는 다음과 같다.

```
current run 1: exit=101  FAILED steer_folds_into_active_turn_without_cancelling
current run 2: exit=101  FAILED steer_folds_into_active_turn_without_cancelling
current run 3: exit=101  FAILED tool_metadata_caps_enforced
parent  run 1: exit=101  FAILED steer_folds_into_active_turn_without_cancelling
parent  run 2: exit=0    (all passed)
parent  run 3: exit=101  FAILED tool_metadata_caps_enforced
```

실패하는 테스트 이름이 양쪽에서 동일하고 실행마다 바뀐다. 원인이 드러난
것은 `tool_metadata_caps_enforced`로, 도구 200개를 노출하는 fake MCP에
대해 `mcp: list_tools many: timeout after 2s`가 발생한다. 하드코딩된 2초
한도가 이 하드웨어에서 부족한 것이다. `cancelled_turn_with_usage_emits_
notification_before_response`와 `steer_folds_into_active_turn_without_
cancelling`도 취소·steering 경합을 다루는 타이밍 의존 테스트다.

**따라서 SchoolX 회귀가 아니라 기존 환경 플래키로 분류한다.**

#### 2026-07-28 재조사 — 원인은 CPU 부하다

세션 B의 `just test`에서 목록 밖 이름(`steer_rejected_on_empty_prompt`)이
나와 위 규칙대로 재조사했다. 격리 실행에서는 재현되지 않았고, **CPU 코어
수만큼 busy loop을 띄운 상태**에서 `cargo test -p buzz-agent --test fake_llm`을
8회씩 돌리자 양쪽에서 재현됐다.

| snapshot | 8회 중 실패 | 나온 이름 |
|---|---:|---|
| 현재 (`schoolx`, 병합 후) | 7 | `steer_folds_into_active_turn_without_cancelling`, `cancelled_turn_with_usage_emits_notification_before_response`, `steer_rejected_on_empty_prompt` |
| 부모 (`acfbb1bb`) | 6 | 위 3종 + `steer_rejected_on_run_id_mismatch` |

부모가 현재 트리에서 나오지 않은 이름까지 실패시켰다. 부하 없는 격리
실행에서는 부모도 5회 중 2회 `steer_folds_into_active_turn_without_cancelling`이
실패했다.

**분류를 개별 테스트 이름에서 파일 단위로 넓힌다.**
`crates/buzz-agent/tests/fake_llm.rs`의 steer·cancel 계열 전체가 이 하드웨어의
CPU 부하에 민감하며, 부모 스냅샷이 최소한 같은 빈도로 실패한다. 이 파일의
테스트 실패는 SchoolX 회귀로 취급하지 않는다. 이 파일 **밖**의 새 실패는
여전히 조사 대상이다.

이 계열이 실패했을 때는 부하 없는 상태에서 해당 테스트만 재실행해 확인한다.

```bash
cargo test -p buzz-agent --test fake_llm
```

### 아직 미검증

### i18n Playwright smoke

`ab3af828` 병합 이후 상태에서 실제 앱 부팅 순서로 실행했다.

```bash
pnpm --dir desktop build:e2e                                        # exit 0
pnpm --dir desktop exec playwright test tests/e2e/i18n.spec.ts --project=smoke
# 4 passed (16.0s), exit 0
```

fresh-install `en-US`→영어, 미지원 `ja-JP`→한국어, 한·영 양방향 전환,
새로고침 후 저장값 유지를 덮는다. missing-key 영어 fallback E2E는 아직
없으며 Phase 2 잔여 작업이다.

### 라이선스 보존

| 항목 | 결과 |
|---|---|
| `LICENSE` | Apache License 2.0. `acfbb1bb` 대비 **변경 없음**(diff 없음) |
| `NOTICE` | upstream `block/buzz`에 존재하지 않는다. 보존할 대상이 없으므로 Apache-2.0 §4(d)는 해당 없음 |
| 서드파티 | `cargo-deny check` exit 0 — `advisories ok, bans ok, licenses ok, sources ok` |

`cargo-deny`는 hermit에 포함돼 있어 별도 설치 없이 `. ./bin/activate-hermit`
후 실행된다. yanked crate 경고(`spin 0.9.8`, mesh-llm 경유)가 있으나 upstream
에서 넘어온 것이고 deny 정책상 실패가 아니다.

**아직 충족되지 않은 배포 의무가 하나 있다.** Apache-2.0 §4(b)는 수정한
파일에 변경 사실을 명시하도록 요구한다. SchoolX는 Buzz를 수정해 배포할
계획이므로, 배포본에 "이 소프트웨어는 block/buzz를 수정한 것"이라는 고지가
필요하다. 이는 Phase 0의 보존 확인과는 별개이며 Phase 6(패키징)의
"오픈소스 고지 화면 검증"에서 처리한다.

## upstream 동기화

정책은 `DEVELOPMENT_PLAN.md` §10에 있다. 요약하면 **merge**로 받고
(rebase는 push 전 로컬 브랜치에만), **주 1회 + 각 세션 시작 전**에 한다.

| 항목 | 값 |
|---|---|
| 마지막 확인 SHA | `3a4bf513df0e0c258587bfcbed9463d63723b56b` |
| 확인 일시 | 2026-07-28 (세션 C 시작 전) |
| 받은 커밋 수 | 2 (`925a9a7b`부터) |
| 병합 전 SchoolX tip | `06822959` (`schoolx-pre-upstream-sync-20260728-1550` 브랜치로 보존) |
| 텍스트 충돌 | 없음 |
| 병합 커밋 | `545d1d4e` |

다음 동기화 때 이 표의 SHA를 갱신한다.

### 롤백 브랜치 이름은 분 단위다

이 동기화에서 `schoolx-upstream-merge`의 롤백 브랜치가 **하루 한 개**라
같은 날 두 번째 동기화가 `schoolx-pre-upstream-sync-20260728`을 재사용하려다
"이미 있으니 그냥 둔다"로 넘어갔다. 그 브랜치는 `b9425960`, 즉 세션 A가
끝난 시점을 가리키고 있었으므로 레시피가 안내한
`git reset --hard schoolx-pre-upstream-sync-20260728`은 이번 머지뿐 아니라
**세션 B와 동기화 도구까지 되돌렸을 것이다.**

이후 롤백 브랜치는 `schoolx-pre-upstream-sync-<YYYYMMDD-HHMM>`을 쓰고,
이름이 이미 다른 커밋에 잡혀 있으면 레시피가 실패한다. 접미사는 여전히
사전순으로 정렬되므로 `schoolx-upstream-check`의 "가장 최근 브랜치" 기본값이
그대로 동작한다.

이 동기화의 검사 3(제품 식별자)은 stale 브랜치를 기준으로 삼는 바람에 이번
머지가 가져온 파일이 아니라 세션 A 이후 244개 파일을 훑었다. 범위가 넓어진
쪽이라 결과 자체는 유효하다.

### 2026-07-28 동기화에서 드러난 것

텍스트 충돌은 lockfile 하나였지만 **자동 병합된 파일에서 조용한 충돌이
하나 나왔다.** upstream이 `0025_relay_invites.sql`을 추가하면서 SchoolX의
`0025_restrict_managed_agent_channel_add_policy.sql`과 마이그레이션 버전
번호가 겹쳤다. sqlx는 `_sqlx_migrations`를 version으로 키잉하면서도 중복
버전을 컴파일 타임에 거부하지 않는다 — 이 상태에서 `cargo build -p buzz-db`는
통과했고, 개발 DB는 version 25를 SchoolX 마이그레이션으로 이미 기록하고 있어
upstream의 `relay_invites`가 **영구히 적용되지 않는** 상태였다.

이후 SchoolX 전용 마이그레이션은 **예약 대역 `9001+`** 를 쓴다. 자세한 이유는
[`PRODUCT_IDENTITY.md`](PRODUCT_IDENTITY.md) §5.

개발 DB는 통째로 초기화하지 않고 잘못된 원장 한 줄만 제거해 복구했다.
SchoolX 마이그레이션이 idempotent한 `UPDATE`라 재적용이 안전하다.

```bash
psql "$DATABASE_URL" -c \
  "DELETE FROM _sqlx_migrations WHERE version = 25 \
   AND description = 'restrict managed agent channel add policy';"
cargo run -p buzz-admin -- migrate
```

`cargo run -p buzz-admin -- migrate`는 `.env`를 읽지 않는다. `DATABASE_URL`을
셸에서 명시적으로 export해야 컨테이너(5433)에 붙는다. 안 하면 호스트
Homebrew postgres(5432)에 붙어 `role "buzz" does not exist`로 죽는다.

전체 stdout 로그는 세션 로컬 경로에 있었고 repo에 보존하지 않았다. 위
표의 명령을 그대로 재실행하면 같은 값을 얻는다.

## 필수 비교 명령

원본과 현재 snapshot에서 아래 명령을 같은 순서와 toolchain으로 실행한다.

```bash
git status --short
pnpm --dir desktop typecheck
pnpm --dir desktop check
pnpm --dir desktop test
pnpm --dir desktop build
just desktop-tauri-test
```

마지막 항목은 `cargo test --manifest-path desktop/src-tauri/Cargo.toml`이
아니다. 그 형태는 sidecar stub 선행 단계를 건너뛰어 깨끗한 checkout에서
반드시 실패한다.

i18n 변경에는 추가로 다음을 실행한다.

```bash
pnpm --dir desktop build:e2e
pnpm --dir desktop exec playwright test tests/e2e/i18n.spec.ts --project=smoke
```

Playwright의 repo 설정에서 test path 해석이 다르면 `desktop` 디렉터리에서
동일한 smoke project와 `i18n.spec.ts`를 지정하고, 실제 사용한 명령을
표에 그대로 기록한다.

relay, database, auth를 수정한 뒤에는 `just test`를 별도 행으로 추가한다.
Postgres·Redis가 필요하므로 아래 환경 준비를 먼저 읽는다.

## 통합 테스트 환경 준비

`just test`는 `buzz-postgres`·`buzz-redis` 컨테이너를 요구한다.
`_ensure-services`는 두 컨테이너가 healthy면 아무 작업도 하지 않으므로,
포트를 바꿔 올려둔 상태를 그대로 재사용한다.

```bash
colima start                 # Docker Desktop 대신 사용 중인 데몬
cp .env.example .env         # 없으면 생성. .gitignore에 등록돼 있다
docker compose up -d postgres redis
```

**살아있는 relay가 필요한 e2e에는 MinIO도 올린다.** relay는 기동 중
git object-store 정합성 프로브(A3 gate)를 돌리고, S3 백엔드가 없으면
`git conformance probe failed: s3 backend error`로 **중단된다**. Postgres와
Redis만으로는 relay가 뜨지 않는다.

```bash
docker compose up -d minio minio-init
```

`just test-e2e`는 이 두 컨테이너를 스스로 올리므로 수동 실행 시에만 필요하다.

**호스트 포트 5432 충돌에 주의한다.** 이 환경에는 Homebrew
`postgresql@17`이 brew service로 떠 있고 `127.0.0.1:5432`를 명시적으로
바인딩한다. colima는 포트를 `*:5432` 와일드카드로 포워딩하므로 두
데몬이 충돌 없이 공존하지만, `localhost` 접속은 더 구체적인 바인딩이
이겨서 **컨테이너가 아니라 호스트 postgres로 연결된다.** 증상은
`role "buzz" does not exist`이고, `docker compose ps`는 healthy로
보이기 때문에 로그만으로는 드러나지 않는다.

호스트 서비스를 끄는 대신 Buzz를 다른 포트로 올린다.

```yaml
# compose override — repo 밖에 두고 -f 로 명시 전달한다
services:
  postgres:
    ports:
      - "5433:5432"
```

```bash
docker compose -f docker-compose.yml -f <override>.yml up -d postgres redis
```

`.env`의 `DATABASE_URL`과 `PGPORT`를 5433으로 맞춘다. 접속 대상이
컨테이너인지는 서버 문자열로 확인한다 — 컨테이너는 musl 빌드다.

```bash
psql "postgres://buzz:buzz_dev@localhost:5433/buzz" -tAc "select version();"
# → PostgreSQL 17.10 on aarch64-unknown-linux-musl
```

## 원본 snapshot 재실행 절차

현재 작업 디렉터리를 checkout으로 바꾸지 않는다. clean detached
worktree를 만들고 결과를 기록한다.

```bash
BASELINE_WORKTREE="$(mktemp -d /tmp/schoolx-buzz-baseline.XXXXXX)"
git worktree add --detach "${BASELINE_WORKTREE}" acfbb1bb6af54cb29cb152496ff43b8285dcb8cf
cd "${BASELINE_WORKTREE}"
. ./bin/activate-hermit
pnpm install --frozen-lockfile
```

`just desktop-tauri-test`를 실행하기 전에 sherpa-onnx 아카이브 위치를
지정한다. 지정하지 않으면 build script가 GitHub release에서 받으려 하고,
네트워크가 막힌 환경에서는 DNS 실패로 죽는다.

```bash
export SHERPA_ONNX_ARCHIVE_DIR=<main-checkout>/desktop/src-tauri/target/sherpa-onnx-prebuilt
```

의존성 설치 명령과 network/cache 조건도 기록한다. 모든 필수 비교 명령을
마친 뒤 repo root로 돌아와 `git worktree list`에서 정확한 임시 경로를
확인하고 worktree를 제거한다. 결과 기록과 필요한 로그를 옮기기 전에는
제거하지 않는다.

## 결과 기록 형식

각 실행마다 다음 값을 보존한다.

- commit SHA와 `git status --short`
- 시작·종료 시각과 timezone
- 정확한 명령
- exit code
- passed, failed, skipped 수
- 실패한 test 또는 lint rule 이름
- infrastructure 실패인지 product 실패인지
- cargo/vite 캐시가 cold인지 warm인지
- 로그 또는 CI run의 보존 위치

파이프로 결과를 요약할 때 종료 코드가 마지막 명령의 것으로 바뀌지 않게
한다. `cmd | tail`은 `tail`의 exit code를 남기므로 판정에 쓸 수 없다.

권장 표 형식:

| snapshot | command | exit | passed | failed | skipped | 분류 | 로그 |
|---|---|---:|---:|---:|---:|---|---|
| `<sha>` | `<exact command>` | `<code>` | `<n>` | `<n>` | `<n>` | 기존/SchoolX/infra | `<path or run URL>` |

## Phase 0 완료 게이트

다음 조건이 모두 충족되기 전에는 Phase 0을 완료로 표시하지 않는다.

- [x] parent와 foundation snapshot에서 필수 비교 명령을 모두 실행했다.
- [x] 모든 결과에 commit, 도구 버전, 시각, exit code가 있다.
- [x] 기존 실패와 SchoolX 회귀가 분리돼 있다.
- [x] upstream fetch·merge 또는 rebase 정책과 마지막 확인 SHA가 기록됐다.
- [x] i18n Playwright 테스트가 실제 앱 부팅 순서로 실행됐다.
- [x] Apache-2.0, NOTICE, third-party license 보존 여부가 확인됐다.

6개가 모두 충족됐다. **Phase 0 완료.**

Phase 0이 닫혔다는 것은 "원본과 현재를 같은 기준으로 비교할 수 있다"는
뜻이지 "모든 테스트가 통과한다"는 뜻이 아니다. 위에 기록한 `buzz-agent`
플래키 3종은 여전히 실패하며, 그것이 기존 실패임을 증명한 것이 이 단계의
결과물이다. 다음 세션은 `IMPLEMENTATION_HANDOFF.md`의 세션 A(보안·호환성
계약)로 진행한다.
