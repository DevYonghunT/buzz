# 세션 핸드오프 (2026-08-07)

새 세션이 여기부터 이어간다. 프로젝트 전반의 상시 문서는
[`IMPLEMENTATION_HANDOFF.md`](IMPLEMENTATION_HANDOFF.md), 직전 이틀은
[`SESSION_HANDOFF_20260805.md`](SESSION_HANDOFF_20260805.md)이고, 이 문서는
**8/6–8/7에 무엇이 끝났고 무엇이 열려 있는지**만 다룬다.

## 0. 첫 5분에 할 것

```bash
cd /Users/kim-yonghun/Development/schoolX_v2.0   # 워크트리 아님 — §6 참조
. ./bin/activate-hermit
git status -sb          # 깨끗해야 한다
```

**전부 푸시되어 있다** (`9ce69a5e`). 다음 작업은 §2.1.

앱을 띄워 볼 일이 있으면 **릴레이를 먼저** 켜야 한다(§3.1). Colima가 꺼져
있으면 `colima start`가 선행한다.

## 1. 이번에 끝난 것

| 주제 | 내용 | 커밋 |
|---|---|---|
| 팀 빌드 | 첫 실행 성공, macOS 사이드카 버그 수정, Intel 잡 추가 | `93da132a`–`7de99dde` |
| 사용 설명서 | 한국어 PDF 17쪽 + 스크린샷 스펙 + 빌드 스크립트 | `93e3f1d2` |
| 키 굽는 빌드 | 관리자가 API 키를 구운 설치본을 만드는 경로 | `aaf53eb9`–`8c4f960a` |
| 온보딩 축소 | 값이 구워지면 하네스·모델 화면을 건너뛴다 | `cdd41b4f` |
| 한글화 | 첫 동선 전체 + 사이드바 | `d7a23112`–`e8705b7b` |
| 문서 작성 | HWPX 등 도구를 고정한 스킬을 앱에 내장 | `9ce69a5e` |

**팀원 첫 경험은 이제 이렇다**: 설치 → 경고 넘기기 → 프라이빗키 만들기 →
커뮤니티 연결. 하네스·모델·API 키 단계가 전부 사라졌고 화면은 한국어다.

## 2. 지금 열려 있는 것 — 우선순위 순

### 2.1 릴레이 서버 구축 ← **다음 작업, 다른 모든 것의 병목**

**서버를 확보했다.** 테스트용 서브도메인을 받았고, 잘 되면 AWS로 옮길
계획이다. 아직 아무것도 올리지 않았다.

지금은 **선생님 맥에서 `just relay`로 띄운 릴레이가 곧 서버**다. 터미널을
닫으면 팀 전체가 멈춘다. 이게 배포를 막는 진짜 이유다.

**반드시 지킬 조건 — 최종 호스트명을 지금 고정한다.**

릴레이는 **접속 호스트 문자열로 커뮤니티를 가른다**(§3.2). 테스트 서버가 준
서브도메인을 그대로 쓰다가 AWS로 옮기면 주소가 바뀌고, 그건 **다른
커뮤니티**다 — 채널·메시지·멤버십이 전부 옛 주소에 남는다.

> **정정** — 초판은 여기에 "주소를 빌드에 구우면 주소 변경은 **전원 재설치**를
> 뜻한다"고 적었으나 코드는 그렇지 않다. 구운 주소는 잠금이 아니라 **기본값**이고
> (`desktop/src-tauri/src/relay.rs:43` — `workspace override > env > build-time >
> default`), 앱에 커뮤니티 추가·전환·삭제가 있다(`desktop/src/app/App.tsx:305`).
> 주소가 바뀌면 팀원은 **앱에서 커뮤니티를 추가**하면 되고 재설치는 필요 없다.
> 옛 커뮤니티의 데이터가 그 주소에 남는다는 것은 그대로 사실이다.
>
> 이 정정 때문에 **버려도 되는 테스트라면 빌린 서브도메인으로 시작해도 된다.**
> 호스트명 고정은 운영 배포에서 지킬 규칙이다.

> 사용자 소유 도메인으로 `relay.<학교주소>` 하나를 만들고, 지금은 그 DNS를
> 테스트 서버로, 나중에 AWS로 **DNS만 옮긴다.** 호스트 문자열이 안 바뀌니
> 커뮤니티도 배포한 설치본도 살아 있다.

같이 잡아야 할 것:

- **TLS 필수** (`wss://`). 자체 서명 인증서면 팀원 앱이 붙지 못한다
- **프로세스 관리자로 상시 실행** (systemd 등). 재부팅해도 살아나야 한다
- **주소 표기 통일** — `wss://relay.x.kr`과 `wss://1.2.3.4`도 서로 다른 커뮤니티다

서버에서 아래를 돌린 결과가 있으면 구성을 바로 시작할 수 있다:

```bash
cat /etc/os-release | head -2; uname -m; nproc; free -h | head -2; docker --version
```

필요한 것: Postgres, Redis, (파일 업로드를 쓰면) MinIO. `deploy/compose`와
`deploy/charts`에 자산이 있다.

### 2.2 팀원 승인 — 수단은 이미 있다

**릴레이 멤버십(NIP-43)**이 구현되어 있다. 스위치는
`BUZZ_REQUIRE_RELAY_MEMBERSHIP=true`이고 `RELAY_OWNER_PUBKEY`·
`BUZZ_RELAY_PRIVATE_KEY`와 **셋이 한 묶음**이다(하나라도 빠지면 릴레이가
기동을 거부한다 — 8/5 핸드오프 §3.2).

들어오는 길 두 가지:

| 방법 | 명령/동작 | 성질 |
|---|---|---|
| 관리자 직접 승인 | `buzz-admin add-member <pubkey> --role member` | 정확하지만 사람마다 손이 간다 |
| 초대 코드 | 관리자가 발급 → 팀원이 입력 → `relay_members`에 자동 등록 | HMAC 서명, 무상태. **만료 전까지 재사용 가능**(`crates/buzz-relay/src/invite_token.rs`) |

초대 코드는 "used" 표시가 없으므로 **사람마다 따로, 짧은 만료로** 발급한다.
유출이 의심되면 릴레이 키쌍을 교체하면 발급된 코드가 전부 무효가 된다.

**주의**: `.env`에 멤버십을 상시로 켜 두면 `just test-e2e`가 0.24초 만에
전멸한다(8/5 핸드오프 §3.1). 릴레이 실행에만 얹는다.

### 2.3 한글화 — 첫 동선은 끝, 앱 내부가 남음

| 화면 | 상태 |
|---|---|
| 랜딩 / 프라이빗키 생성·백업 / 커뮤니티 연결 / 이름 / 아바타 | **완료** |
| 사이드바 상단 내비 + 섹션 제목 | **완료** |
| 채널·메시지 본문, 컨텍스트 메뉴, 다이얼로그, 오류 문구 | 남음 (81개 파일) |
| 하네스·모델 화면 | **불필요** — 구운 빌드에서 안 나온다 |

**기본 로케일이 이미 `ko`**이고 fallback만 `en`이다
(`desktop/src/shared/i18n/locale.ts`). 즉 문구를 키로 옮기는 즉시 한국어로
보인다 — 옮긴 만큼 바로 효과가 난다.

번역하며 정한 기준(이어서 할 때 톤을 맞출 것):

- **용어는 「프라이빗키」** — 사용자가 정했다. 사용 설명서도 같이 바꿨다
- **내비 이름은 음차** — 인박스·펄스. 뜻을 설명하는 문구가 아니라 화면
  위치를 가리키는 고유 이름에 가깝고, 팀원끼리 그렇게 부르게 된다. 뜻이
  분명한 것만 우리말로(Direct messages → 개인 메시지)
- **문장에 제품명을 넣지 않는다** — 워드마크는 라틴 문자 `SchoolX`인데
  본문만 「스쿨엑스」로 쓰면 한 화면에서 두 표기가 부딪힌다. 주어를
  생략하는 편이 한국어로도 자연스럽다
- **문장 속 링크는 조각으로 나눈다** — 자리표시자 하나로 두면 한국어에서
  링크 뒤에 붙어야 할 조사가 영어 어순에 갇힌다

남은 81개 파일은 **대부분 가끔 마주치는 화면**이다. §2.4 피드백으로 실제로
막히는 곳을 보고 우선순위를 정하는 편이 효율이 높다.

### 2.4 온보딩 사람 검증 — 여전히 열려 있음

8/5 핸드오프 §2.2 그대로다. 소스를 보지 않은 팀원이 설치본만으로 들어가는지
확인해야 한다. **§2.1이 끝나야 실제로 가능하다** — 지금은 릴레이가 로컬이라
팀원이 붙을 수 없다.

### 2.5 매뉴얼 그림이 어긋나기 시작했다

`docs/schoolx-2/manual/`의 그림은 E2E 하네스가 **영어로** 렌더한 것인데 앱은
이제 한국어로 뜬다. 매뉴얼 본문의 "화면이 영어로 보이는 것은 정상입니다"도
곧 거짓이 된다.

스크린샷 스펙에 `appLocale` 옵션이 있으므로, 한글화가 일단락되면
`installMockBridge(page, undefined, { appLocale: "ko" })`로 **16장을 한 번에
다시 뽑아** PDF를 재생성한다. 낱장으로 갈아끼우면 영어·한국어가 섞여 더
혼란스럽다.

매뉴얼 5장(모델·API 키 설정)도 **구운 빌드에서는 통째로 불필요**하므로,
배포 방식이 확정되면 "이미 설정되어 있습니다"로 줄인다.

### 2.6 앱 이름이 아직 Buzz다

`CFBundleName`이 `Buzz`라 macOS 메뉴 막대에 그렇게 뜨고, 실행 파일 이름도
`buzz`다. 번들 폴더만 `SchoolX.app`이다. 랜딩 화면 워드마크도 아직 Buzz
이미지다(`/landing/buzz-wordmark.png`). 8/5 핸드오프 §2.5의 로고 항목과 같은
계열이지만, 이름 쪽은 디자인 결정이 아니라 설정값이라 지금도 고칠 수 있다.

## 3. 이번에 알게 된 것 중 다음 세션이 밟기 쉬운 함정

### 3.1 릴레이는 터미널에 매달린 포그라운드 프로세스다

`just relay`를 띄운 터미널을 닫으면 릴레이가 죽는다. 그러면 앱은
**Connecting/Reconnecting**을 반복하고, 스레드가 "Some message context could
not be loaded"로 뜨며, **사람 이름이 pubkey로 보인다**(프로필 kind:0을 못
읽어서). 캐시된 메시지 본문은 남아 있어 글은 보이므로 앱 버그처럼 오인하기
쉽다.

**증상 셋이 함께 나오면 릴레이부터 확인한다**:

```bash
lsof -nP -iTCP:3000 -sTCP:LISTEN   # 비어 있으면 릴레이가 없는 것
```

### 3.2 `localhost`와 `127.0.0.1`은 서로 다른 커뮤니티다

**오늘 가장 오래 걸린 문제다.** 봇을 채널에 추가했는데 아무 응답이 없었고,
멤버십은 DB의 모든 계층에서 정상이었다.

릴레이는 **접속 호스트 문자열로 커뮤니티를 가른다**(`communities` 테이블이
host → community_id를 담는다). 앱은 `localhost:3000`으로 붙어 있었고
에이전트는 `ws://127.0.0.1:3000`으로 붙어 있었다. 두 커뮤니티가 갈려서
에이전트는 **빈 커뮤니티를 정상적으로 조회**하고 있었다.

```
155baa19… | localhost:3000  | 이벤트 2558개  ← 앱
b5c36cdd… | 127.0.0.1:3000  | 이벤트 1개     ← 에이전트
```

증상은 에이전트 로그의 `discovered 0 channel(s)` / `no channel subscriptions
resolved — agent will sit idle`이고, 릴레이 로그에는 **인증 성공 + 200 +
`result_count=0`**으로 찍힌다. 오류가 아니라 정상 응답이라 더 안 보인다.

진단에 쓴 것:

```bash
docker exec buzz-postgres psql -U buzz -d buzz -c "SELECT host, id FROM communities;"
tail -f /tmp/relay.log | grep '"route":"/query"'   # just relay 2>&1 | tee /tmp/relay.log
```

**앱이 localhost로 붙는데 에이전트만 127.0.0.1을 받는 지점은 아직 못
찾았다.** 저장된 레코드에는 `ws://localhost:3000`이 들어 있고
(`managed-agents.json`), 코드도 "레코드 값은 무시하고 활성 워크스페이스
릴레이를 쓴다"(`effective_agent_relay_url`)로 되어 있다. 워크스페이스 릴레이가
어디서 `127.0.0.1`로 바뀌는지가 남은 숙제다. 코드에
`loopback_ws_localhost_preserves_authority`라는 테스트가 있는 것을 보면 이미
한 번 겪은 함정이다.

**배포에서 그대로 재발한다** — 도메인 대 IP 사이에서 같은 일이 난다. §2.1의
호스트명 고정이 근본 대책이다.

### 3.3 키 굽기는 크레이트가 다시 컴파일돼야 반영된다

`scripts/schoolx-keyed-build.sh`로 만든 첫 설치본에 **키가 들어가지
않았다.** build.rs는 세 값을 정상적으로 내보냈는데(PROVIDER, MODEL, 168자
블롭) 바이너리에는 없었다 — cargo가 데스크톱 크레이트를 프레시로 판단해
재컴파일을 건너뛰었고 tauri는 이전 바이너리를 그대로 번들했다.

`cargo:rustc-env`은 **크레이트가 실제로 다시 컴파일될 때만** 바이너리에
닿는다. 스크립트가 이제 `option_env!`가 든 파일을 touch해 재확장을 강제하고,
**빌드 끝에 바이너리를 직접 검사해** 키가 없으면 실패시킨다. 성공하면
마지막 줄에 이렇게 나온다:

```
verified: the baked provider, model, and key are present in the binary.
```

**그 줄이 없으면 설치하지 않는다.** 없으면 조용히 키 없는 설치본이 나가고,
팀원은 설정 화면의 빈 API 키 칸을 보고도 원인을 짚을 단서가 없다.

### 3.4 굽는다고 남용이 막히지는 않는다

구운 키는 바이너리 안 base64라 `strings`와 디코드로 복구된다. UI 마스킹은
화면 표시에 관한 것이지 바이너리를 지키지 않는다. **이 포크는 공개
저장소**이므로 구운 설치본을 Actions 아티팩트나 Release로 올리면 사실상 키를
공개하는 것이다.

- 구운 설치본은 **USB·에어드랍·사내 드라이브로 직접** 건넨다
- **한도를 걸고 폐기 가능한 전용 키**를 쓴다
- GitHub Secret은 *소스*에 키가 남는 것만 막지 *빌드 결과물*은 못 막는다 —
  자주 혼동되는 지점이다

사용자가 택한 방향: **진짜 키를 굽고 한도로 관리**, 팀이 커지면 게이트웨이
토큰으로 이전.

### 3.5 빌드 스크립트가 저장소를 더럽힌다 (막아 뒀음)

`set-version-from-tag.mjs`는 추적 중인 네 파일의 버전을 고쳐 쓴다. 한 번
`git add -A`로 그게 커밋에 딸려 들어가 저장소 버전이 `0.5.3` →
`0.5.3-team.local`이 됐다. 스크립트가 EXIT trap으로 복구하게 했고
`tauri.team.conf.json`은 `.gitignore`에 넣었다. **빌드 후 `git status`를
한 번 보는 습관이 여전히 낫다.**

### 3.6 python-hwpx의 표 처리 (문서화 안 된 것)

`doc.text.replace()`는 **표 안의 글자에 닿지 않는다**(0 반환). 표는
`doc.tables.fill_by_path({"라벨>right": "값"})`을 써야 하고, 경로 구분자는
`>`다(방향은 `right`/`left`/`up`/`down`). 오류 메시지는 `path must include at
least one direction` 뿐이라 소스를 봐야 알 수 있었다. 6.0에서
`export_text`→`text.plain`, `replace_text_in_runs`→`text.replace`로 옮겨갔고
옛 이름은 7.0에서 사라진다. 전부
`desktop/src-tauri/src/managed_agents/nest_document_skill.md`에 적혀 있다.

## 4. 상태 요약

```
브랜치: codex/schoolx-2-foundation  (저장소 기본 브랜치)
원격:   9ce69a5e — 로컬과 동기
워킹트리: 깨끗
```

이번 세션 게이트: 데스크톱 단위 테스트 3,935개, Tauri 2,079개, typecheck,
biome, `check-i18n-formatters`, `desktop-tauri-fmt` 전부 통과. Rust 워크스페이스
전체와 mobile·E2E는 이 세션에서 돌리지 않았다 — push 훅이 대부분을 덮는다.

**팀에 나눠줄 수 있는 것**: [run 30999329683](https://github.com/DevYonghunT/buzz/actions/runs/30999329683)의
설치본 3종(키 없음)과 `docs/schoolx-2/manual/SchoolX-사용설명서.pdf`. 키를
구운 설치본은 `scripts/schoolx-keyed-build.sh`로 로컬에서 만든다.

## 5. 검증된 사실 (다시 확인하지 말 것)

- **에이전트는 로컬에서 실제로 일한다** — `buzz-dev-mcp`가 `shell`,
  `read_file`, `str_replace`, `view_image`를 제공한다. 바탕화면에 파일을
  만들고 웹에서 자료를 받아오는 것까지 확인했다. 기본 작업 폴더는
  `~/.schoolx`, 없으면 홈
- **승인 창이 없다** — `session/request_permission`을 **자동 승인**한다
  (`crates/buzz-acp/src/acp.rs:1148`). Claude Code와 결정적으로 다른 점이다.
  에이전트를 "Anyone"으로 열면 커뮤니티의 누구나 그 맥에서 셸 명령을 돌릴 수
  있고, 앱이 그 화면에서 직접 경고한다
- **HWPX는 한글 프로그램 없이 된다** — `python-hwpx` 6.0.2로 생성·서식
  채우기·검증까지 확인했고, 만든 파일이 실제 한글에서 정상으로 열렸다
- **shared compute(`relay-mesh`)는 이 빌드에서 못 쓴다** —
  `shared compute is not available in this build`. 매뉴얼에 고르지 말라고
  적어 뒀다
- **`TAURI_BUNDLER_DMG_IGNORE_CI`는 팀 빌드에 불필요하다** — 8/5 핸드오프
  §3.0. 다른 macOS 잡 셋이 켜므로 누락으로 보이지만 아니다

## 6. 작업 환경 주의

- **메인 체크아웃에서 작업한다** (`/Users/kim-yonghun/Development/schoolX_v2.0`).
  워크트리에서는 `just desktop-tauri-fmt`가 실패해 pre-commit이 막힌다
- **hermit을 먼저 활성화한다** — `. ./bin/activate-hermit`. 안 하면
  `command not found: just`
- **Colima가 Docker다** — 꺼져 있으면 `colima start`. `just relay`가
  Postgres·Redis 컨테이너를 필요로 한다
- `git commit -s` 필수 (DCO)
- **`git add -A` 앞에서 `git status`를 본다** (§3.5)
- **10분 한도**: 오래 걸리는 명령은 백그라운드로. push도 pre-push 훅 때문에
  넘길 수 있다
