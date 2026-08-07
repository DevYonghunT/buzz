# 릴레이 서버 배포 핸드오프

서버 배포를 맡으실 분을 위한 문서입니다. 이 저장소를 처음 보신다는 전제로
썼습니다.

**저장소**: `School-X520/schoolx` (비공개) — 작업 브랜치 `codex/schoolx-2-foundation`.
`DevYonghunT/buzz`(공개, `block/buzz`의 포크)에도 같은 내용이 올라가 있고 두 곳을
같이 갱신합니다. 배포 작업은 비공개 쪽에서 하세요.

**맡으실 일**: 지금 개발자 노트북에서 포그라운드로 돌고 있는 릴레이를, 팀원들이
바깥에서 붙을 수 있는 상시 서버로 올리는 것.

**지금 상태**: 코드는 다 됐습니다. 배포만 안 됐습니다. 릴레이가 `just relay`로
로컬에서만 돌아서 팀원이 붙을 수 없고, 그게 나머지 전부를 막고 있습니다.

---

## 1. 이게 무엇인가

[Buzz](https://github.com/block/buzz)의 포크입니다. Nostr 프로토콜 기반 팀 협업
도구이고, 학교용으로 고쳐 쓰고 있습니다.

**릴레이**는 그 중심의 서버입니다. HTTP API가 아니라 **WebSocket 상시 연결**이
주 통신 방식입니다 — 앱이 접속해서 연결을 계속 열어 두고 실시간으로 이벤트를
주고받습니다. 그래서 서버리스(Vercel, Lambda 등)에는 올릴 수 없습니다.

| 구성 요소 | 무엇 |
|---|---|
| relay | Rust 바이너리. WebSocket + 좁은 HTTP 표면(NIP-11, `/events`, `/query`, Blossom 미디어, git smart HTTP, 헬스) |
| Postgres 17 | 이벤트 저장소. 메시지 본문이 여기 들어갑니다 |
| Redis 7 | pub/sub 팬아웃, presence, 타이핑 인디케이터 |
| MinIO | S3 호환 미디어 저장소 (첨부파일) |

클라이언트는 Tauri 2 + React 데스크톱 앱입니다. 릴레이와 별개로 배포되고 이미
빌드 파이프라인이 있습니다.

전체 구조는 [ARCHITECTURE.md](../../ARCHITECTURE.md)에 있습니다.

---

## 2. 시작하기 전에 반드시 아셔야 할 세 가지

이 셋을 모르고 시작하면 시간을 크게 버립니다.

### 2.1 공개 이미지를 쓸 수 없습니다 ← 가장 먼저

`deploy/compose/.env.example:6`의 기본값이 이렇습니다:

```
BUZZ_IMAGE=ghcr.io/block/buzz:main
```

**이건 upstream Buzz 이미지라 이 포크에 쓰면 안 됩니다.** 포크가 릴레이 쪽을
1,180줄 고쳤습니다:

```
crates/buzz-core/src/audience.rs        (신규 210줄)
crates/buzz-core/src/kind.rs            (+62)
crates/buzz-db/src/migration.rs         (+47)
crates/buzz-relay/src/handlers/ingest.rs (+325)
crates/buzz-relay/src/state.rs          (+131)
crates/buzz-relay/src/handlers/auth.rs  (+96)
... 외 12개 파일
migrations/..._restrict_managed_agent_channel_add_policy.sql (신규)
```

upstream 이미지를 올리면 audience 규칙, 에이전트 채널 정책, 신규 이벤트 kind가
통째로 빠집니다. 조용히 잘못 도는 종류의 실패입니다.

**이미지 만드는 법 — 세 가지 중 택일**

| 방법 | 어떻게 | 비고 |
|---|---|---|
| **CI (권장)** | 저장소 변수 `GHCR_IMAGE`를 `ghcr.io/school-x520/schoolx`로 설정하고 `relay-v0.1.0` 같은 태그를 푸시 | `.github/workflows/docker.yml:79`가 이 변수를 읽습니다. 태그 트리거는 브랜치와 무관하게 동작하고, amd64·arm64 멀티아치를 네이티브 러너에서 빌드합니다. **GHCR 이름은 소문자여야 합니다** |
| 로컬 빌드 후 푸시 | 루트 `Dockerfile`로 `docker buildx build --platform <서버아키텍처>` | 서버와 아키텍처가 다르면 QEMU라 매우 느립니다 |
| 서버에서 직접 빌드 | 서버에 소스를 놓고 `docker build` | Rust 워크스페이스라 무겁습니다. RAM 4GB 미만이면 실패합니다 |

CI 경로가 제일 깔끔합니다. `docker.yml`은 `push: branches: [main]`과
`tags: ["relay-v[0-9]*"]`에 걸려 있는데, 이 포크의 작업 브랜치는
`codex/schoolx-2-foundation`이라 **브랜치 푸시로는 안 돕니다.** 태그를 써야
합니다.

### 2.2 `RELAY_URL`이 커뮤니티 ID를 만듭니다

이게 이 시스템에서 제일 놀라운 성질입니다.

릴레이는 멀티테넌트입니다. **접속 호스트 문자열**로 커뮤니티를 가릅니다
(`communities` 테이블이 host → community_id를 담습니다). 호스트를 뽑아내는
경로는 이렇습니다:

```
RELAY_URL  →  relay_url_authority()  →  normalize_host()  →  커뮤니티 host
             (crates/buzz-relay/src/main.rs:261)
```

읽는 환경변수는 **`RELAY_URL`** 입니다 (`crates/buzz-relay/src/config.rs:477`).

> **함정**: 기동 실패 메시지는 `BUZZ_RELAY_URL`이라고 말합니다
> (`crates/buzz-relay/src/main.rs:264`). **그런 환경변수는 없습니다.**
> 메시지가 틀렸습니다. `RELAY_URL`을 보세요.

**실제로 일어난 사고 (반나절 소요)**

앱은 `localhost:3000`, 에이전트는 `ws://127.0.0.1:3000`으로 붙어 있었습니다.
같은 서버인데 **호스트 문자열이 달라서 커뮤니티가 갈렸습니다**:

```
155baa19… | localhost:3000  | 이벤트 2558개  ← 앱
b5c36cdd… | 127.0.0.1:3000  | 이벤트 1개     ← 에이전트
```

에이전트는 오류 없이 **빈 커뮤니티를 정상 조회**하고 있었습니다. 릴레이 로그에는
인증 성공 + 200 + `result_count=0`으로 찍혀서 더 안 보입니다.

**배포에서 그대로 재발합니다** — 도메인 대 IP, `www` 유무, 포트 표기 차이 전부
같은 일을 일으킵니다. **주소 표기를 한 가지로 고정하세요.**

진단 명령:

```bash
docker exec <postgres컨테이너> psql -U buzz -d buzz -c "SELECT host, id FROM communities;"
```

### 2.3 멤버십은 세 값이 한 묶음입니다

팀원 승인(NIP-43)을 켜려면 셋을 **다 같이** 줘야 합니다. 하나라도 빠지면
릴레이가 기동을 거부합니다:

| 환경변수 | 검증 위치 |
|---|---|
| `BUZZ_REQUIRE_RELAY_MEMBERSHIP=true` | — |
| `RELAY_OWNER_PUBKEY` (64자 hex) | `crates/buzz-relay/src/main.rs:228` |
| `BUZZ_RELAY_PRIVATE_KEY` (64자 hex) | `crates/buzz-relay/src/main.rs:242` |

추가로 `RELAY_URL`에서 호스트를 뽑을 수 없으면 역시 거부합니다
(`main.rs:261`). 즉 멤버십을 켜면 §2.2가 강제됩니다.

`BUZZ_RELAY_PRIVATE_KEY`는 **재시작해도 같은 값이어야** 합니다. 임시 키로 서명한
NIP-43 이벤트는 재시작 후 검증이 안 됩니다.

> `.env`에 멤버십을 상시로 켜 두면 `just test-e2e`가 0.24초 만에 전멸합니다.
> 릴레이 실행 환경에만 얹으세요.

---

## 3. 배포 자산

이미 준비돼 있습니다. 새로 짜실 필요 없습니다.

```
deploy/compose/
  compose.yml         relay + postgres + redis + minio (+ minio-init)
  compose.caddy.yml   Caddy 리버스 프록시 오버레이 — Let's Encrypt 자동
  compose.dev.yml     DB·MinIO 포트를 호스트에 노출 (디버깅용)
  Caddyfile           {$BUZZ_DOMAIN} { reverse_proxy relay:3000 }
  run.sh              래퍼 — start/stop/status/config/backup-hint
  .env.example        복사해서 CHANGE_ME를 전부 채울 것
  README.md           운영 노트

deploy/charts/buzz/   Helm 차트 (쿠버네티스로 갈 경우)
```

`run.sh`는 `.env`에 `CHANGE_ME`가 남아 있으면 기동을 거부합니다. 의도된
동작입니다.

**단일 노드면 compose로 충분합니다.** Helm 차트는 나중에 쿠버네티스로 옮길 때
쓰세요.

---

## 4. 배포 절차

```bash
cd deploy/compose
cp .env.example .env
$EDITOR .env            # 아래 표 참고, CHANGE_ME 전부 교체
./run.sh config         # 렌더링 확인
BUZZ_COMPOSE_TLS=true ./run.sh start
```

`BUZZ_COMPOSE_TLS=true`가 `compose.caddy.yml`을 얹어 Caddy가 80/443을 잡고
Let's Encrypt 인증서를 자동 발급합니다. **TLS는 필수입니다** — 자체 서명
인증서면 데스크톱 앱이 아예 못 붙습니다.

### 채워야 할 값

| 키 | 값 | 비고 |
|---|---|---|
| `BUZZ_IMAGE` | 포크에서 빌드한 이미지 | §2.1 — **기본값 쓰지 마세요** |
| `BUZZ_DOMAIN` | `relay.<도메인>` | Caddy가 이 이름으로 인증서를 받습니다 |
| `RELAY_URL` | `wss://relay.<도메인>` | §2.2 — 커뮤니티 ID가 여기서 나옵니다 |
| `BUZZ_MEDIA_BASE_URL` | `https://relay.<도메인>/media` | |
| `BUZZ_MEDIA_SERVER_DOMAIN` | `relay.<도메인>` | |
| `BUZZ_CORS_ORIGINS` | `https://relay.<도메인>` | |
| `RELAY_OWNER_PUBKEY` | 관리자 계정의 64자 hex 공개키 | 앱에서 확인 가능 |
| `BUZZ_RELAY_PRIVATE_KEY` | `openssl rand -hex 32` | **한 번 만들고 고정** |
| `BUZZ_GIT_HOOK_HMAC_SECRET` | `openssl rand -hex 32` | 〃 |
| `POSTGRES_PASSWORD` / `REDIS_PASSWORD` | 무작위 | 〃 |
| `BUZZ_S3_ACCESS_KEY` / `BUZZ_S3_SECRET_KEY` | 무작위 | 〃 |
| `BUZZ_AUTO_MIGRATE` | `true` | 첫 기동에 스키마를 만듭니다. 끄면 `buzz-admin migrate`를 먼저 돌리세요 |

`BUZZ_S3_ENDPOINT`와 `BUZZ_S3_ADDRESSING_STYLE`은 `compose.yml`이 고정합니다
(`http://minio:9000` / `path`). Docker DNS가 `<bucket>.minio`를 풀지 못해서
그렇습니다. 외부 S3를 쓰시려면 compose를 고치거나 Helm으로 가세요.

### 네트워크가 NAT 뒤라면

공인 IP가 없거나 80/443을 열 수 없으면 Caddy의 Let's Encrypt(HTTP-01)가 실패
합니다. 그 경우 Cloudflare Tunnel을 쓰세요 — 아웃바운드만으로 붙고 인증서는
Cloudflare가 처리합니다. 단 도메인 네임서버를 Cloudflare로 옮겨야 합니다.

터널을 쓰면 릴레이는 평문 HTTP로 두고(`compose.yml`만, TLS 오버레이 없이)
터널이 `localhost:3000`을 가리키게 하면 됩니다. `RELAY_URL`은 여전히
`wss://relay.<도메인>`입니다.

---

## 5. 검증

```bash
# 1. 헬스 — 컨테이너 안에서
curl -fsS http://127.0.0.1:8080/_liveness
curl -fsS http://127.0.0.1:8080/_readiness
# 엔드포인트: /_liveness, /_readiness, /_status (crates/buzz-relay/src/router.rs:69)

# 2. TLS + NIP-11 — 바깥에서
curl -fsS -H "Accept: application/nostr+json" https://relay.<도메인>/

# 3. 커뮤니티가 의도한 호스트로 하나만 있는지 ← §2.2 확인
docker exec <postgres컨테이너> psql -U buzz -d buzz -c "SELECT host, id FROM communities;"

# 4. WebSocket 업그레이드
curl -i -N -H "Connection: Upgrade" -H "Upgrade: websocket" \
     -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" \
     https://relay.<도메인>/
```

3번에서 호스트가 둘 이상 나오면 §2.2의 사고가 이미 일어난 것입니다.

마지막으로 데스크톱 앱에서 `wss://relay.<도메인>`으로 커뮤니티를 추가해 실제로
붙는지 확인하세요.

---

## 6. 팀원 들이기

멤버십을 켰다면(§2.3) 두 가지 길이 있습니다.

| 방법 | 명령 | 성질 |
|---|---|---|
| 직접 승인 | `buzz-admin add-member <pubkey> --role member` | 정확하지만 사람마다 손이 갑니다. `BUZZ_RELAY_PRIVATE_KEY`가 필요합니다 (`crates/buzz-admin/src/main.rs:395`) |
| 초대 코드 | 관리자가 발급 → 팀원이 입력 → `relay_members`에 자동 등록 | HMAC 서명, 무상태 (`crates/buzz-relay/src/invite_token.rs`) |

**초대 코드에는 "사용됨" 표시가 없습니다.** 만료 전까지 재사용됩니다. 사람마다
따로, 짧은 만료로 발급하세요. 유출이 의심되면 릴레이 키쌍을 갈면 발급된 코드가
전부 무효가 됩니다.

---

## 7. 함정 모음

**릴레이가 죽으면 앱은 이렇게 보입니다** — Connecting/Reconnecting 반복,
스레드가 "Some message context could not be loaded", **사람 이름이 pubkey로
표시**(프로필 kind:0을 못 읽어서). 캐시된 메시지는 남아 있어 글은 보이므로 앱
버그로 오인하기 쉽습니다. **셋이 함께 나오면 릴레이부터 확인하세요.**

**쿼리에 `kinds`가 없으면 403입니다.** p-gate에 걸립니다. 디버깅 중 REQ를 직접
쏘실 때 주의하세요.

**에이전트는 승인 없이 셸 명령을 실행합니다** (`crates/buzz-acp/src/acp.rs:1148`
— `session/request_permission`을 자동 승인). 릴레이 서버에서는 에이전트를 돌리지
마세요. 릴레이만 돌립니다.

**릴레이 로그에서 쿼리 추적**:

```bash
docker logs -f <relay컨테이너> 2>&1 | grep '"route":"/query"'
```

`result_count=0`이 계속 찍히면 §2.2를 의심하세요.

---

## 8. 지금 미결인 것

| 항목 | 상태 |
|---|---|
| **호스트명 확정** | 미정. 도메인을 정해야 시작됩니다 |
| **서버 확보** | 협의 중. 팀원 기계를 빌리는 방향이고, 필요한 정보는 [RELAY_HOST_REQUEST.md](RELAY_HOST_REQUEST.md)로 요청해 뒀습니다 |
| **릴레이 이미지** | 아직 없음. §2.1의 CI 경로를 쓰려면 `GHCR_IMAGE` 저장소 변수 설정이 선행됩니다 |
| **`RELAY_OWNER_PUBKEY`** | 관리자 공개키 미확정 |
| 백업 | 미정. `./run.sh backup-hint` 참고 |
| 모니터링 | 미정 |

**테스트 단계 전제**: 지금 배포는 버려도 되는 테스트입니다. 데이터가 사라져도
됩니다. 주소가 나중에 바뀌면 팀원은 앱에서 커뮤니티를 추가하면 되고 재설치는
필요 없습니다(`desktop/src-tauri/src/relay.rs:43` — 구운 주소는 잠금이 아니라
기본값). 다만 옛 주소의 데이터는 그 커뮤니티에 남습니다.

**운영으로 갈 때 정할 것**: 소유 도메인으로 호스트명 고정, TLS, 백업, 모니터링,
그리고 릴레이 운영자가 모든 메시지 평문을 볼 수 있다는 점에 대한 정책.

---

## 9. 로컬에서 돌려보려면

```bash
cd <저장소루트>
. ./bin/activate-hermit    # 안 하면 `command not found: just`
cp .env.example .env
just setup                 # 의존성 설치 + 마이그레이션
just relay                 # ws://localhost:3000
```

Docker가 필요합니다(개발자 맥에서는 Colima를 씁니다 — 꺼져 있으면
`colima start`). `just relay`는 `cargo run -p buzz-relay`라 **디버그 빌드에
포그라운드**입니다. 터미널을 닫으면 죽습니다. 서버용은 아닙니다.

---

## 10. 참고 문서

| 문서 | 내용 |
|---|---|
| [SESSION_HANDOFF_20260807.md](SESSION_HANDOFF_20260807.md) | 직전 개발 세션. §2.1이 이 배포 작업의 원본 요구사항 |
| [SESSION_HANDOFF_20260805.md](SESSION_HANDOFF_20260805.md) | 그 전 세션. §3.2에 멤버십 3종 세트 최초 기록 |
| [IMPLEMENTATION_HANDOFF.md](IMPLEMENTATION_HANDOFF.md) | 프로젝트 상시 문서 |
| [RELAY_HOST_REQUEST.md](RELAY_HOST_REQUEST.md) | 서버 빌려주는 분에게 보낸 정보 요청 |
| [SELF_HOSTED_ONBOARDING.md](SELF_HOSTED_ONBOARDING.md) | 셀프호스팅 온보딩 |
| [../../ARCHITECTURE.md](../../ARCHITECTURE.md) | 시스템 설계 |
| [../../CONTRIBUTING.md](../../CONTRIBUTING.md) | 개발 환경, PR 절차 |
| [deploy/compose/README.md](../../deploy/compose/README.md) | compose 운영 노트 |

커밋은 `git commit -s` 필수입니다 (DCO 체크가 서명 없는 커밋을 막습니다).
