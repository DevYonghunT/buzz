# SchoolX 2.0 재현 가능한 기준선

이 문서는 “원본 Buzz에서 이미 실패하던 항목”과 “SchoolX 변경으로 생긴
실패”를 구분하기 위한 실행 기록이다. 결과가 없는 항목을 통과로 추정하지
않으며 `미검증`으로 남긴다.

## snapshot

| 구분 | 값 |
|---|---|
| 원본 비교 snapshot | `acfbb1bb6af54cb29cb152496ff43b8285dcb8cf` |
| i18n foundation snapshot | `a3e1ca4fb5f199b8ade62e14c120967ddab190d9` |
| 개발 브랜치 | `codex/schoolx-2-foundation` |
| origin | `https://github.com/DevYonghunT/buzz.git` |
| upstream | `https://github.com/block/buzz.git` |
| 검토 환경 | macOS arm64, Asia/Seoul |
| 기록일 | 2026-07-24 |

원본 비교 snapshot은 foundation commit의 parent다. upstream 최신 상태와
같다는 의미는 아니므로 실행 전 `git fetch upstream` 후 관계를 별도로
기록한다.

## 도구 버전

2026-07-24에 repo Hermit 환경에서 확인한 값이다.

| 도구 | 버전 |
|---|---|
| Node.js | `v24.14.0` |
| pnpm | `11.4.0` |
| rustc | `1.95.0 (59807616e 2026-04-14)` |
| cargo | `1.95.0 (f2d3ce0bd 2026-03-21)` |

모든 실행은 먼저 다음 명령으로 repo toolchain을 활성화한다.

```bash
. ./bin/activate-hermit
```

## 현재 보존된 실행 증거

| snapshot | 명령 | 결과 | 실행일 | 비고 |
|---|---|---|---|---|
| `a3e1ca4f` | `pnpm --dir desktop test` | 통과, 3,407 passed / 0 failed | 2026-07-23 | 전체 desktop 단위 테스트, 약 439.7초 |
| `a3e1ca4f` | `pnpm --dir desktop typecheck` | 미검증 | - | 이전 handoff에는 통과로 적혀 있었으나 이 문서가 참조할 로그가 없음 |
| `a3e1ca4f` | `pnpm --dir desktop check` | 미검증 | - | 동일 |
| `a3e1ca4f` | `pnpm --dir desktop build` | 미검증 | - | 동일 |
| `a3e1ca4f` | `cargo test --manifest-path desktop/src-tauri/Cargo.toml` | 미검증 | - | root workspace test에 포함되지 않음 |
| `a3e1ca4f` | i18n Playwright smoke | 미검증 | - | test 파일은 있으나 이 검토에서 재실행하지 않음 |
| `acfbb1bb` | 아래 필수 desktop 명령 전체 | 미검증 | - | 변경 전 실행 기록이 보존되지 않았으므로 Phase 0 미완료 |

“type 검사, 정적 검사, build가 통과했다”는 과거 요약만으로 재현 가능한
baseline이 되지는 않는다. 실행 로그 또는 최소한 exit code, commit,
명령, 시각을 이 표에 남긴 뒤에만 `통과`로 바꾼다.

## 필수 비교 명령

원본과 현재 snapshot에서 아래 명령을 같은 순서와 toolchain으로 실행한다.

```bash
git status --short
pnpm --dir desktop typecheck
pnpm --dir desktop check
pnpm --dir desktop test
pnpm --dir desktop build
cargo test --manifest-path desktop/src-tauri/Cargo.toml
```

i18n 변경에는 추가로 다음을 실행한다.

```bash
pnpm --dir desktop build:e2e
pnpm --dir desktop exec playwright test tests/e2e/i18n.spec.ts --project=smoke
```

Playwright의 repo 설정에서 test path 해석이 다르면 `desktop` 디렉터리에서
동일한 smoke project와 `i18n.spec.ts`를 지정하고, 실제 사용한 명령을
표에 그대로 기록한다.

relay, database, auth를 수정하는 이후 단계에서는 repo 지침에 따라
`just test`와 필요한 Postgres·Redis 환경도 별도 행으로 추가한다.

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

의존성 설치 명령과 network/cache 조건도 기록한다. 모든 필수 비교 명령을 마친 뒤 repo root로
돌아와 `git worktree list`에서 정확한 임시 경로를 확인하고 worktree를
제거한다. 결과 기록과 필요한 로그를 옮기기 전에는 제거하지 않는다.

## 결과 기록 형식

각 실행마다 다음 값을 보존한다.

- commit SHA와 `git status --short`
- 시작·종료 시각과 timezone
- 정확한 명령
- exit code
- passed, failed, skipped 수
- 실패한 test 또는 lint rule 이름
- infrastructure 실패인지 product 실패인지
- 로그 또는 CI run의 보존 위치

권장 표 형식:

| snapshot | command | exit | passed | failed | skipped | 분류 | 로그 |
|---|---|---:|---:|---:|---:|---|---|
| `<sha>` | `<exact command>` | `<code>` | `<n>` | `<n>` | `<n>` | 기존/SchoolX/infra | `<path or run URL>` |

## Phase 0 완료 게이트

다음 조건이 모두 충족되기 전에는 Phase 0을 완료로 표시하지 않는다.

- parent와 foundation snapshot에서 필수 비교 명령을 모두 실행했다.
- 모든 결과에 commit, 도구 버전, 시각, exit code가 있다.
- 기존 실패와 SchoolX 회귀가 분리돼 있다.
- i18n Playwright 테스트가 실제 앱 부팅 순서로 실행됐다.
- upstream fetch·merge 또는 rebase 정책과 마지막 확인 SHA가 기록됐다.
- Apache-2.0, NOTICE, third-party license 보존 여부가 확인됐다.

현재 상태는 이 조건을 충족하지 않으므로 **Phase 0 미완료**다.
