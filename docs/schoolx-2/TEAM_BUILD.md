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

두 잡이 병렬로 돈다. 끝나면 Actions 실행 페이지 하단 **Artifacts**에 둘이 붙는다.

| 아티팩트 | 내용 |
|---|---|
| `schoolx-macos-<version>` | `.dmg` |
| `schoolx-windows-<version>` | NSIS `.exe` |

버전은 `desktop/package.json`의 값에 `-team.<실행번호>`를 붙인 것이다. 올리지
않고 접미사만 붙이는 이유는, 테스터의 응용 프로그램 폴더에서 정식 릴리스와
구별되지 않으면 곤란하기 때문이다.

**아티팩트는 유효기간이 있다**(GitHub 기본 90일, 조직 설정에 따라 더 짧을 수
있다). 오래 두고 쓸 링크가 아니므로 받는 즉시 팀에 공유한다.

## 2. 팀원에게 줄 때 — 이 안내를 같이 보낸다

미서명 빌드라 **두 OS 모두 처음 열 때 경고가 뜬다.** 안내 없이 파일만 주면
"바이러스라는데요"라는 답이 온다.

### macOS

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
| macOS 서명 | **없음** — 경고 우회 필요 | Apple Developer 계정으로 서명·공증 |
| Windows 서명 | **없음** — SmartScreen 경고 | 코드 서명 인증서 |
| 자동 업데이트 | **없음** (`createUpdaterArtifacts: false`) | 업데이트 endpoint |
| 배포 경로 | Actions 아티팩트 (한시적) | GitHub Release |

서명은 코드 작업이 아니라 **구매**다. macOS는 Apple Developer Program(연
$99), Windows는 코드 서명 인증서가 필요하다. 학교에 배포하기 전에는 최소한
macOS 서명이 있어야 한다 — 교사가 우클릭·시스템 설정을 거쳐야 하는 앱은
설치 단계에서 이탈한다.

## 5. 왜 `release.yml`을 안 쓰는가

`.github/workflows/release.yml`은 이 포크에서 **돌지 않는다.** 모든 잡이
`if: github.repository == 'block/buzz'`로 막혀 있고, macOS 잡은
`block/apple-codesign-action`으로 Block의 Apple 계정에 OIDC로 붙는다. 태그를
찍어 GitHub Release와 자동 업데이트 아티팩트까지 만드는 흐름이라, 팀원 몇
명에게 파일을 건네는 목적과 맞지 않는다.

그래서 `windows-canary.yml`의 모양(빌드 → 아티팩트 업로드, 태그·릴리스 없음)을
따르고 macOS 잡을 더했다. macOS는 `release.yml`이 서명 액션에 넘기기 직전
단계인 `--no-sign`에서 멈춘다 — 그 차이가 전부다.
