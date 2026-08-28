# 팀 피드백용 설치본 만들기·나눠주기

내부 피드백을 받기 위한 **미서명** macOS·Windows 설치본을 뽑는 절차다.
학교에 배포하는 정식 빌드가 아니다 — 차이는 §4에 적었다.

## 1. 뽑기

GitHub Actions에서 **SchoolX Team Build** 워크플로를 수동 실행한다
(`.github/workflows/schoolx-team-build.yml`, `workflow_dispatch`).

```bash
gh workflow run schoolx-team-build.yml --repo DevYonghunT/buzz
gh run watch --repo DevYonghunT/buzz
```

이 명령이 워크플로를 찾는 것은 **저장소 기본 브랜치가
`codex/schoolx-2-foundation`이기 때문이다.** `workflow_dispatch`는 워크플로
파일이 기본 브랜치에 있을 때만 트리거된다. 기본 브랜치를 `main`(upstream
미러)으로 되돌리면 이 파일은 기본 브랜치에서 사라지고, 실행이 실패하는 게
아니라 **워크플로 목록에 아예 뜨지 않는다.**

세 잡이 병렬로 돈다. 끝나면 Actions 실행 페이지 하단 **Artifacts**에 셋이 붙는다.

| 아티팩트 | 받을 사람 | 내용 |
|---|---|---|
| `schoolx-macos-apple-silicon-<version>` | M1 이후 Mac | `.dmg` |
| `schoolx-macos-intel-<version>` | Intel Mac | `.dmg` |
| `schoolx-windows-<version>` | Windows | NSIS `.exe` |

> **macOS Actions 아티팩트는 SchoolX Code를 실행할 수 없다.** 두 DMG는
> Developer ID 서명 전 중간 산출물이므로, SchoolX Code의 Git XPC 상호 인증이
> 이를 거부한다. Gatekeeper에서 「열기」를 허용하거나 앱을 응용 프로그램으로
> 옮겨도 해결되지 않는다. Mac에서 SchoolX Code를 테스트할 설치본은 반드시
> §4.1의 서명·공증 후처리를 통과한 DMG만 공유한다.

Mac이 둘로 갈리는 이유는 `macos-latest` 러너가 Apple Silicon이라 그냥 빌드하면
arm64 전용이 나오기 때문이다. Intel 잡은 `--target x86_64-apple-darwin`으로
교차 컴파일한다.

버전은 `desktop/package.json`의 값에 `-team.<실행번호>`를 붙인 것이다. 올리지
않고 접미사만 붙이는 이유는, 테스터의 응용 프로그램 폴더에서 정식 릴리스와
구별되지 않으면 곤란하기 때문이다.

**아티팩트는 유효기간이 있다**(GitHub 기본 90일, 조직 설정에 따라 더 짧을 수
있다). 오래 두고 쓸 링크가 아니므로 받는 즉시 팀에 공유한다.

## 2. 팀원에게 줄 때 — 이 안내를 같이 보낸다

미서명 빌드라 **두 OS 모두 처음 열 때 경고가 뜬다.** 안내 없이 파일만 주면
"바이러스라는데요"라는 답이 온다.

### macOS

**먼저 둘 중 어느 파일인지 고른다.** 화면 왼쪽 위 **애플 메뉴 → 이 Mac에
관하여**를 연다. 「칩」이 보이면 `...-apple-silicon-...`, 「프로세서」에
Intel이 보이면 `...-intel-...`이다.

> 잘못 받으면 앱이 **아예 실행되지 않는다.** 그런데 그 증상이 아래 미서명
> 경고와 구별되지 않아서, 안내를 따라 우클릭·시스템 설정을 아무리 해도
> 열리지 않는 상태로 막힌다. 파일을 건넬 때 어느 쪽인지 같이 알려주는 게
> 가장 확실하다.

1. `.dmg`를 열고 앱을 **응용 프로그램**으로 끈다.
2. 처음 실행은 **앱을 우클릭 → 열기** — 더블클릭하면 열 방법이 없는
   경고만 뜬다.
3. 대화상자에서 **열기**를 누른다. 두 번째부터는 그냥 열린다.

최신 macOS는 우클릭으로도 막힐 수 있다. 그때는 **시스템 설정 → 개인정보
보호 및 보안**을 열면 하단에 "SchoolX을(를) 열도록 허용"이 나온다.

### Windows

1. `.exe`를 실행하면 **"Windows의 PC 보호"** 파란 창이 뜬다.
2. **추가 정보** → **실행**.

## 3. 되돌리기

테스트를 끝내고 지울 때:

- macOS: 응용 프로그램에서 앱 삭제. 데이터는 `~/.schoolx`와
  `~/Library/Application Support/io.github.schoolx520.app`에 남으므로 완전히
  지우려면 둘 다 삭제한다.
- Windows: 설정 → 앱에서 제거.

## 4. 정식 배포와 무엇이 다른가

| | 팀 빌드 (이 문서) | 정식 배포 |
|---|---|---|
| macOS 서명 | **Actions 결과에는 없음** — 승인 운영자 후처리는 §4.1 | Apple Developer 계정으로 서명·공증 |
| Windows 서명 | **없음** — SmartScreen 경고 | 코드 서명 인증서 |
| macOS SchoolX Code | **미서명 아티팩트에서는 사용 불가** — §4.1 후처리 필요 | 사용 가능 |
| 자동 업데이트 | **없음** (`createUpdaterArtifacts: false`) | 업데이트 endpoint |
| 배포 경로 | Actions 아티팩트 (한시적) | GitHub Release |

서명은 코드 작업이 아니라 **구매**다. macOS는 Apple Developer Program(연
$99), Windows는 코드 서명 인증서가 필요하다. 학교에 배포하기 전에는 최소한
macOS 서명이 있어야 한다 — 교사가 우클릭·시스템 설정을 거쳐야 하는 앱은
설치 단계에서 이탈한다.

### 4.1 승인된 운영자가 macOS DMG를 후처리할 때

Actions에서 받은 미서명 macOS DMG는 Team `3WPS7QNZV5`의 기존 Developer ID
Application identity와 검증된 `notarytool` keychain profile이 있는 Mac에서
다음 스크립트로 후처리할 수 있다. 실제 identity나 credential은 명령행·문서·
로그에 쓰지 않고, 승인된 shell 환경의 `DEVELOPER_ID_IDENTITY`와
`NOTARY_PROFILE`에만 둔다.

```bash
./scripts/schoolx-sign-notarize-team-dmg.sh \
  --arch arm64 \
  --input /absolute/path/SchoolX_apple-silicon_unsigned.dmg \
  --output /absolute/path/SchoolX_apple-silicon_signed.dmg

./scripts/schoolx-sign-notarize-team-dmg.sh \
  --arch x86_64 \
  --input /absolute/path/SchoolX_intel_unsigned.dmg \
  --output /absolute/path/SchoolX_intel_signed.dmg
```

스크립트는 원본을 바꾸거나 기존 출력을 덮어쓰지 않는다. 다섯 sidecar → Code
Git XPC → 앱 순으로 서명하고 앱을 별도로 공증·staple한 뒤, 그 앱으로 DMG를
재조립한다. 마지막 DMG byte도 다시 서명·공증·staple하고 Gatekeeper와 저장소
release verifier가 모두 통과한 뒤에만 출력 경로에 공개한다. `arm64`와
`x86_64`는 각각 해당 Actions 아티팩트에 지정해야 하며 architecture 불일치는
즉시 실패한다.

이 후처리는 팀 테스트용 DMG의 macOS 경고를 없애는 절차다. Team Build는 계속
자동 업데이트 아티팩트를 만들지 않으며, 이 로컬 결과는 보호된 태그와 정식
release provenance를 가진 canonical 배포본을 대신하지 않는다.

## 5. API 키를 구워서 배포하기 (선택)

팀원마다 API 키를 넣게 하는 대신, **관리자가 키를 구운 설치본**을 만들어
건넬 수 있다. 그러면 팀원은 설치하고 열쇠만 만들면 끝이고, 사용 설명서
5장(모델·키 설정)을 밟지 않아도 된다.

```bash
ANTHROPIC_API_KEY="$(op read op://private/anthropic/key)" \
SCHOOLX_MODEL=<모델-id> \
  ./scripts/schoolx-keyed-build.sh
```

Intel용은 `SCHOOLX_TARGET=x86_64-apple-darwin`을 얹어 한 번 더 돌린다.
`SCHOOLX_RELAY_URL`을 주면 릴레이 주소까지 미리 박혀 팀원이 주소를 입력할
일도 없다.

구운 값은 **가장 낮은 우선순위**라 팀원이 설정에서 덮어쓸 수 있고, 설정
화면에는 `(inherited from build)`로 표시되며 키는 `••••••`로 가려진다.
`SCHOOLX_MODEL`은 기본값을 두지 않았다 — Anthropic은 모델이 필수라
(`config: ANTHROPIC_MODEL required`) 잘못 추측한 기본값을 구우면 모든
팀원의 빌드가 조용히 망가진다.

> **구운 설치본은 절대 Actions 아티팩트나 Release로 올리지 않는다.**
> 키는 바이너리 안에 base64로 들어 있어 `strings`와 디코드로 복구된다. UI
> 마스킹은 화면 표시에 관한 것이지 바이너리를 지키지 않는다. 이 포크는
> **공개 저장소**이므로 아티팩트로 올리면 사실상 키를 공개하는 것이다.
> USB·에어드랍·사내 드라이브로 직접 건네고, **한도를 건 폐기 가능한 키**를
> 쓴다. GitHub Secret에 넣는 것은 *소스*에 키가 남는 것만 막지 *빌드
> 결과물*은 막지 못한다 — 둘은 별개다.

키를 구운 빌드에서도 팀원은 여전히 자기 열쇠(identity key)를 직접 만든다.
그건 신원이지 자격증명이 아니라 공유 대상이 아니다.

## 6. 왜 `release.yml`을 안 쓰는가

`.github/workflows/release.yml`은 이 포크에서 **돌지 않는다.** 모든 잡이
`if: github.repository == 'block/buzz'`로 막혀 있고, macOS 잡은
`block/apple-codesign-action`으로 Block의 Apple 계정에 OIDC로 붙는다. 태그를
찍어 GitHub Release와 자동 업데이트 아티팩트까지 만드는 흐름이라, 팀원 몇
명에게 파일을 건네는 목적과 맞지 않는다.

그래서 `windows-canary.yml`의 모양(빌드 → 아티팩트 업로드, 태그·릴리스 없음)을
따르고 macOS 잡을 더했다. macOS는 `release.yml`이 서명 액션에 넘기기 직전
단계인 `--no-sign`에서 멈춘다 — 그 차이가 전부다.
