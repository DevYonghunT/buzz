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

소요 시간은 cargo/vite 캐시 상태에 좌우된다. 위 값은 원본 worktree가
cold, 현재 checkout이 warm인 상태에서 측정했으므로 성능 비교에 쓰지
않는다.

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

**따라서 SchoolX 회귀가 아니라 기존 환경 플래키로 분류한다.** 다만 표본이
작다. 원본은 3회 중 1회 완전 통과했고 현재는 3회 모두 실패했으므로,
"현재가 더 나쁘지 않다"까지 주장하지는 않는다. 새로운 실패 *양상*이
없다는 것까지만 근거가 있다. 이 세 테스트가 앞으로 실패하더라도 이름이
위 목록과 같으면 SchoolX 회귀로 취급하지 않되, 목록 밖의 이름이 나오면
다시 조사한다.

### 아직 미검증

| 항목 | 상태 |
|---|---|
| i18n Playwright smoke | 미검증 — `build:e2e` + smoke project를 이번 수집에서 실행하지 않음 |
| Apache-2.0 / NOTICE / third-party license 보존 | 미검증 |

## upstream 동기화

정책은 `DEVELOPMENT_PLAN.md` §10에 있다. 요약하면 **merge**로 받고
(rebase는 push 전 로컬 브랜치에만), **주 1회 + 각 세션 시작 전**에 한다.

| 항목 | 값 |
|---|---|
| 마지막 확인 SHA | `ab3af828714ab699dfc87644d234014987a4fe6b` |
| 확인 일시 | 2026-07-25 |
| 받은 커밋 수 | 57 (`acfbb1bb`부터) |
| 병합 전 SchoolX tip | `14790a94` (`schoolx-pre-upstream-sync-20260725` 브랜치로 보존) |

다음 동기화 때 이 표의 SHA를 갱신한다.

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
- [ ] i18n Playwright 테스트가 실제 앱 부팅 순서로 실행됐다.
- [ ] Apache-2.0, NOTICE, third-party license 보존 여부가 확인됐다.

6개 중 4개가 충족됐다. 남은 2개가 닫히기 전에는 **Phase 0 미완료**다.
