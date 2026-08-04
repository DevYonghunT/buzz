# 자체 호스팅 릴레이 온보딩 구현 계획 (세션 G)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 자기 릴레이를 띄운 학교 관리자가 소스를 읽지 않고 화면 안내만으로 앱에 들어올 수 있게 한다.

**Architecture:** 연결 경로는 이미 있다(`startConnection` + `InviteRedeemForm`의 bare-relay-URL 제출). 새로 만드는 것은 그 경로로 가는 **문**뿐이다. 새 UI는 SchoolX 소유 파일 하나에 두고, upstream 파일 **하나**에만 렌더 두 줄을 넣어 병합 표면을 최소화한다.

**Tech Stack:** React 19 / Tauri 2 / Playwright

> **실행 완료 (2026-08-05).** 게이트는 [`BASELINE.md`](../BASELINE.md) 세션 G
> 절, 결과 서술은 [`IMPLEMENTATION_HANDOFF.md`](../IMPLEMENTATION_HANDOFF.md)
> 세션 G 절. 커밋 `a465de78`·`1e80e1a7`·`c3d72fa7`.
>
> **계획과 다르게 한 것 셋.**
>
> 1. **upstream 파일이 하나로 끝나지 않았다.** 「막힌 다이얼로그는
>    `WelcomeSetup`이 띄우니 그 파일 하나면 된다」는 판단이 틀렸다. 다이얼로그
>    **밖**에 배치한 링크는 오버레이 블러에 묻히고 클릭도 가로채여
>    (`toBeVisible()` 통과, `click()` 타임아웃), 결국 다이얼로그 안에 넣어야
>    했다. 설계가 원래 잡은 두 파일이 맞았다.
> 2. **컴포넌트를 셋으로 나눴다.** 다이얼로그 소유권을 `WelcomeSetup` 최상위로
>    올려야 중첩이 풀린다 — `SelfHostedRelayDialog`(controlled) ·
>    `SelfHostedRelayLink`(신호만) · `SelfHostedRelayEntry`(카드+자기 다이얼로그).
> 3. **사람 검증(Step 4)을 하지 못했다.** 앱이 이미 커뮤니티에 붙어 있어
>    `community.needsSetup`이 거짓이라 온보딩 화면에 닿지 못했다. 대신
>    Playwright가 그 화면을 결정론적으로 띄우고 스크린샷을 냈다. **소스를 보지
>    않은 사람이 화면만으로 들어갈 수 있는지는 여전히 미검증이다** — 세션 G
>    「넘긴 것」에 남겼다.

## Global Constraints

- 작업 위치는 **메인 체크아웃** `/Users/kim-yonghun/Development/schoolX_v2.0`, 브랜치 `codex/schoolx-2-foundation`.
- 시작 전 `. ./bin/activate-hermit`.
- 데스크톱 텍스트 크기는 rem 토큰만 (`text-base`, `text-sm`, `text-xs`, `text-2xs`, `text-3xs`). 임의 리터럴은 `pnpm check:px-text`가 막는다.
- **i18n 키는 기존 네임스페이스 `app` 아래에 넣는다.** 새 네임스페이스를 만들면 `en`·`ko`·`APP_I18N_NAMESPACES`를 한 번에 바꿔야 하고, 빠뜨리면 fallback이 구제하지 못해 한국어에 원시 키가 노출된다(세션 C 사실 1번).
- `en`과 `ko`를 **한 번에** 바꾼다.
- **upstream 파일은 `WelcomeSetup.tsx` 하나만 건드리고, 변경은 import 한 줄 + 렌더 두 줄로 제한한다.** 상태 추가·페이지 분기·카드 재배치 금지, `WelcomeSetupPage` 유니온 확장 금지 — 이 파일은 upstream이 계속 고친다(#2738, #2862). `HostedCommunityOnboarding.tsx`는 건드리지 않는다.
- Playwright 스펙은 `pnpm test:e2e:smoke`로 돈다. `pnpm run build`는 mock bridge를 뗀다.
- 스펙: [`SELF_HOSTED_ONBOARDING.md`](../SELF_HOSTED_ONBOARDING.md).

## 시작 상태 (2026-08-05)

- `InviteRedeemForm`의 `canSubmit`은 `normalizedRelayUrl !== null`만으로 참. `normalizedRelayUrl`은 `onConnect && (!parsed || …)`일 때 계산되므로 `variant="default"` + bare URL이면 활성된다.
- `normalizeRelayUrl`은 `ws://`·`wss://`를 받고 `http://`·`https://`를 승격한다.
- `WelcomeSetup`의 `startConnection(relayUrl)`이 `communityOnboarding.start({source:"first-community", relayUrl})`를 부른다.
- `WelcomeSetupPage` = `"welcome" | "existing" | "join" | "member" | "owned"`. `existing` 페이지의 두 선택지 testid는 `existing-choice-owner`·`existing-choice-member`.
- 막힘이 일어난 다이얼로그는 `WelcomeSetup.tsx:321`이 `isHostedSignInOpen`으로 띄우는 `HostedCommunityOnboarding`이다(`owned` 페이지가 아니다). 그 컴포넌트는 `onBack`·`onReady`만 받고 `onConnect`는 없다.
- 로케일 최상위 네임스페이스: `app`, `settings`, `time`, `appearance`, `catalog`.

---

## File Structure

| 파일 | 책임 |
|---|---|
| `desktop/src/features/communities/ui/SelfHostedRelayEntry.tsx` | **신규, SchoolX 소유** — 문구·다이얼로그·폼 재사용 전부 |
| `desktop/src/shared/i18n/locales/{en,ko}.ts` | `app.selfHostedRelay.*` |
| `desktop/src/features/communities/ui/WelcomeSetup.tsx` | **upstream, 유일** — `existing` 카드 한 줄 + 호스팅 다이얼로그 옆 탈출구 한 줄 |
| `desktop/tests/e2e/self-hosted-onboarding.spec.ts` | **신규** — 두 진입점 + 제출 |
| `desktop/playwright.config.ts` | smoke 등록 |

---

## Task 1: 문구와 컴포넌트

**Files:**
- Create: `desktop/src/features/communities/ui/SelfHostedRelayEntry.tsx`
- Modify: `desktop/src/shared/i18n/locales/en.ts`, `desktop/src/shared/i18n/locales/ko.ts`

**Interfaces:**
- Consumes: `InviteRedeemForm` (`features/onboarding/ui/InviteRedeemForm`), `Dialog` 계열 (`shared/ui/dialog`)
- Produces: `SelfHostedRelayEntry({ onConnect, variant })`

- [x] **Step 1: i18n 키를 양쪽에 더한다**

`en.ts`의 `app` 블록 안에 추가한다.

```ts
    selfHostedRelay: {
      cardTitle: "I run my own relay",
      cardDescription:
        "Connect to a relay your school runs. No invite code needed.",
      link: "Running your own relay?",
      dialogTitle: "Connect to your relay",
      dialogDescription:
        "Enter the address of the relay your school runs. You do not need an invite code — the first administrator to connect is already an owner.",
      placeholder: "ws://relay.our-school.example",
    },
```

`ko.ts`의 `app` 블록 같은 자리에 추가한다.

```ts
    selfHostedRelay: {
      cardTitle: "학교 릴레이를 직접 운영합니다",
      cardDescription:
        "학교가 직접 띄운 릴레이 주소로 연결합니다. 초대 코드가 필요 없습니다.",
      link: "직접 운영하는 릴레이가 있나요?",
      dialogTitle: "릴레이에 연결",
      dialogDescription:
        "학교가 운영하는 릴레이 주소를 입력하세요. 초대 코드는 필요 없습니다 — 처음 연결하는 관리자가 이미 소유자입니다.",
      placeholder: "ws://relay.our-school.example",
    },
```

**`app`에 넣는다.** 새 네임스페이스를 만들지 않는 이유는 Global Constraints에 있다.

"초대 코드가 필요 없습니다"가 이 작업의 핵심 문장이다 — 실제로 막힌 원인이 초대가 필요하다는 오해였다.

- [x] **Step 2: 컴포넌트를 만든다**

```tsx
import React from "react";
import { useTranslation } from "react-i18next";

import { InviteRedeemForm } from "@/features/onboarding/ui/InviteRedeemForm";
import { Card } from "@/shared/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/shared/ui/dialog";

/**
 * "학교 릴레이를 직접 운영합니다" — 자체 호스팅 관리자를 위한 진입점.
 *
 * **연결 경로를 새로 만들지 않는다.** 릴레이 주소만으로 붙는 것은 이미
 * `InviteRedeemForm`이 한다(`canSubmit`이 `normalizedRelayUrl !== null`만으로
 * 참이다). 이 컴포넌트가 더하는 것은 그 경로로 가는 **문과 문구**다.
 *
 * 왜 별도 파일인가: 이것을 부르는 `WelcomeSetup`은 SchoolX가 건드리지 않은
 * upstream 파일이고 upstream이 계속 고친다. 상태와 마크업을 전부 여기 두고
 * 저쪽에는 렌더 줄만 남겨 동기화 충돌 표면을 줄인다. 설계 근거:
 * `docs/schoolx-2/SELF_HOSTED_ONBOARDING.md` §3.
 *
 * 문구는 「소유자/구성원」이 아니라 「릴레이를 직접 운영하는가」로 가른다 —
 * 자체 호스팅 관리자가 자기를 알아보는 유일한 표현이고, 기존 두 선택지가
 * 전부 전자로 이름 붙어 있어 그가 지나쳐 버리는 것이 이 작업의 원인이다(§2).
 */
export function SelfHostedRelayEntry({
  onConnect,
  variant = "card",
}: {
  /** 릴레이 URL 제출. `WelcomeSetup`의 `startConnection`을 그대로 받는다. */
  onConnect: (relayWsUrl: string) => void;
  /**
   * `card` — 선택지 목록 안에 놓이는 카드.
   * `link` — 호스팅 다이얼로그 푸터에 놓이는 한 줄. 이미 들어와 버린
   * 사람의 탈출구라 시각적 무게를 낮춘다.
   */
  variant?: "card" | "link";
}) {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = React.useState(false);

  return (
    <>
      {variant === "card" ? (
        <Card asChild variant="textured">
          <button
            className="flex w-full flex-col items-start gap-1 p-6 text-left"
            data-testid="self-hosted-relay-card"
            onClick={() => setIsOpen(true)}
            type="button"
          >
            <span className="font-medium">
              {t("app.selfHostedRelay.cardTitle")}
            </span>
            <span className="text-sm text-foreground/70">
              {t("app.selfHostedRelay.cardDescription")}
            </span>
          </button>
        </Card>
      ) : (
        <button
          className="text-xs underline underline-offset-2 hover:no-underline"
          data-testid="self-hosted-relay-link"
          onClick={() => setIsOpen(true)}
          type="button"
        >
          {t("app.selfHostedRelay.link")}
        </button>
      )}

      <Dialog onOpenChange={setIsOpen} open={isOpen}>
        <DialogContent data-testid="self-hosted-relay-dialog">
          <DialogTitle>{t("app.selfHostedRelay.dialogTitle")}</DialogTitle>
          <DialogDescription>
            {t("app.selfHostedRelay.dialogDescription")}
          </DialogDescription>
          {/*
            `variant="default"` 이어야 한다. `add-community`는
            `normalizedRelayUrl` 계산 조건이 달라(`hasInviteRelay` 분기)
            bare URL 제출이 막힌다. `onRedeem`은 이 화면에서 쓰이지 않지만
            필수 prop이라 no-op을 넘긴다 — 초대 코드를 넣으면 그건 기존
            초대 화면이 할 일이다.
          */}
          <InviteRedeemForm
            error={null}
            isRedeeming={false}
            onCancel={() => setIsOpen(false)}
            onConnect={(relayWsUrl) => {
              setIsOpen(false);
              onConnect(relayWsUrl);
            }}
            onRedeem={() => {}}
            placeholder={t("app.selfHostedRelay.placeholder")}
            variant="default"
          />
        </DialogContent>
      </Dialog>
    </>
  );
}
```

**확인된 사실.** `Card`는 `asChild`와 `variant="textured"`를 받는다
(`shared/ui/card.tsx`). dialog 모듈은 `Dialog`·`DialogContent`·
`DialogDescription`·`DialogTitle`만 내보내며 **`DialogHeader`는 없다** — 위
코드가 그에 맞춰져 있다.

- [x] **Step 3: 검증한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && pnpm --dir desktop typecheck && pnpm --dir desktop check && pnpm --dir desktop test`
Expected: 전부 PASS. i18n parity 테스트가 `en`/`ko` 구조 일치를 확인한다.

- [x] **Step 4: 커밋한다**

```bash
git add desktop/src/features/communities/ui/SelfHostedRelayEntry.tsx desktop/src/shared/i18n
git commit -s -m "feat(schoolx-2): 세션 G — 자체 호스팅 릴레이 진입점 컴포넌트"
```

---

## Task 2: 두 막다른 곳에 문을 낸다

**Files:**
- Modify: `desktop/src/features/communities/ui/WelcomeSetup.tsx`
- Modify: `desktop/src/features/communities/ui/HostedCommunityOnboarding.tsx`

**둘 다 upstream 파일이다. 변경은 import 한 줄 + 렌더 한 줄로 끝낸다.**

- [x] **Step 1: `existing` 페이지에 세 번째 선택지를 넣는다**

`WelcomeSetup.tsx`의 `existing-choice-member` 카드 **바로 뒤**, 같은 `<div>` 안에 넣는다.

```tsx
                <SelfHostedRelayEntry onConnect={startConnection} />
```

import를 더한다.

```tsx
import { SelfHostedRelayEntry } from "./SelfHostedRelayEntry";
```

`startConnection`은 이미 그 스코프에 있다(`onConnect={startConnection}`으로 아래 join/member 페이지가 쓴다). **`WelcomeSetupPage` 유니온을 건드리지 않는다** — 새 페이지를 만들지 않고 다이얼로그로 처리하는 이유가 이것이다.

- [x] **Step 2: 호스팅 다이얼로그에 탈출구를 넣는다**

**같은 파일이다.** 오늘 실제로 갇힌 그 다이얼로그는 `owned` 페이지가 아니라
`WelcomeSetup.tsx:321`이 `isHostedSignInOpen`으로 띄우는 것이다. 그래서
`HostedCommunityOnboarding.tsx`는 **건드리지 않는다** — upstream 파일 하나로
끝난다.

그 렌더 블록 옆에 링크 형태를 함께 띄운다.

```tsx
          {isHostedSignInOpen && page !== "owned" ? (
            <div className="pointer-events-auto fixed inset-x-0 bottom-8 z-50 flex justify-center">
              <SelfHostedRelayEntry onConnect={startConnection} variant="link" />
            </div>
          ) : null}
```

**다이얼로그 위에 떠야 한다.** 사용자가 마지막으로 보는 줄이 "Builderlab hosts
the relay"이고 그 화면에서 빠져나갈 길이 없는 것이 오늘의 막힘이었다. 배치가
다이얼로그에 가려지면 이 작업은 실패한 것이다 — Step 4의 사람 검증이 그것을
잡는다.

`HostedCommunityOnboarding`에 prop을 더하는 방법도 있으나, 그 파일은
`onBack`·`onReady`만 받는 upstream 원본이고 굳이 늘릴 이유가 없다.

- [x] **Step 3: 검증한다**

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && . ./bin/activate-hermit && pnpm --dir desktop typecheck && pnpm --dir desktop check`
Expected: exit 0

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0 && git diff --stat desktop/src/features/communities/ui/WelcomeSetup.tsx`
Expected: **10줄 미만이고 이 파일 하나뿐**. `HostedCommunityOnboarding.tsx`가
diff에 나오면 설계에서 벗어난 것이다.

- [x] **Step 4: 커밋한다**

```bash
git add desktop/src/features/communities/ui
git commit -s -m "feat(schoolx-2): 세션 G — 막다른 두 곳에서 자체 호스팅 경로로 나갈 수 있다"
```

---

## Task 3: 스펙, 문서, 게이트

**Files:**
- Create: `desktop/tests/e2e/self-hosted-onboarding.spec.ts`
- Modify: `desktop/playwright.config.ts`
- Modify: `docs/schoolx-2/{IMPLEMENTATION_HANDOFF,BASELINE}.md`, `CONTRIBUTING.md`

- [x] **Step 1: 스펙을 쓴다**

```ts
import { expect, test } from "@playwright/test";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

/**
 * 자체 호스팅 릴레이 진입점.
 *
 * 이 스펙이 지키는 것은 연결 로직이 아니라 **문이 있다는 사실**이다. 릴레이
 * 주소로 붙는 경로는 이전부터 동작했고(`InviteRedeemForm`의 bare-URL 제출),
 * 학교 관리자가 그 경로를 찾지 못한다는 것이 문제였다
 * (`docs/schoolx-2/SELF_HOSTED_ONBOARDING.md` §2).
 *
 * 그래서 단언은 「보이는가」와 「누르면 폼이 나오는가」다.
 */

test("the existing-community page offers a self-hosted door", async ({
  page,
}) => {
  await installMockBridge(page, {}, { skipCommunitySeed: true });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  await page.getByTestId("community-choice-existing").click();
  await expect(page.getByTestId("self-hosted-relay-card")).toBeVisible();

  await page.getByTestId("self-hosted-relay-card").click();
  await expect(page.getByTestId("self-hosted-relay-dialog")).toBeVisible();

  await waitForAnimations(page);
});

test("the hosted dialog has a way out to a self-hosted relay", async ({
  page,
}) => {
  await installMockBridge(page, {}, { skipCommunitySeed: true });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  await page.getByTestId("community-choice-existing").click();
  await page.getByTestId("existing-choice-owner").click();

  // 이 다이얼로그가 오늘 실제로 갇힌 지점이다. 앞의 안내를 놓친 사람에게
  // 남은 유일한 출구이므로, 다이얼로그가 열린 상태에서 보여야 한다.
  await expect(page.getByTestId("self-hosted-relay-link")).toBeVisible();

  await waitForAnimations(page);
});
```

**온보딩 화면에 도달하는 방법을 실행 중에 확정한다.** `installMockBridge`의
세 번째 인자에 `skipCommunitySeed`가 있다(`helpers/bridge.ts`). 커뮤니티가
시드되어 있으면 온보딩이 아니라 앱 본화면이 뜨므로 이 옵션이 필요하다.
동작하지 않으면 `skipOnboardingSeed`와 조합하거나, 기존 `onboarding.spec.ts`가
그 화면에 도달하는 방식을 그대로 따른다.

- [x] **Step 2: 스펙을 등록하고 돌린다**

`playwright.config.ts`의 `smoke` `testMatch`에 더한다.

```ts
        "**/self-hosted-onboarding.spec.ts",
```

Run: `cd /Users/kim-yonghun/Development/schoolX_v2.0/desktop && . ../bin/activate-hermit && pnpm test:e2e:smoke self-hosted-onboarding`
Expected: 2 passed

- [x] **Step 3: 판별력을 실증한다**

`WelcomeSetup.tsx`의 렌더 한 줄을 임시로 지우고 첫 테스트가 실패하는지 확인한
뒤 되돌린다. 두 번째도 `HostedCommunityOnboarding.tsx` 쪽으로 같게 한다.
보고서에 적는다.

- [x] **Step 4: 사람이 안내만 보고 들어간다**

**이 작업의 진짜 완료 기준이다.** `just dev`로 앱을 띄우고, 소스를 보지 않은
상태로 화면 문구만 따라 `ws://localhost:3000`까지 도달한다. 도달하지 못하면
문구를 고친다 — 스펙이 초록인 것과 사람이 들어갈 수 있는 것은 다르다.

경로 둘 다 확인한다: `existing` → 카드, 그리고 호스팅 다이얼로그 → 링크.

- [x] **Step 5: 문서를 갱신한다**

- `CONTRIBUTING.md`의 「Getting into the app against your local relay」를
  고친다 — 이제 우회로 설명이 아니라 **화면 안내**를 따라가면 된다. 다만
  `ws://localhost:3000`이라는 값 자체는 남긴다.
- `IMPLEMENTATION_HANDOFF.md`의 「아직 구현 또는 검증되지 않은 것」에서 이
  항목을 빼고, 세션 G 절을 A·B·D·D2·D3·E1 형식으로 더한다.
- `BASELINE.md`에 게이트 실행 기록을 더한다.

- [x] **Step 6: 전체 게이트를 돌린다**

구성 레시피 14개를 하나씩 포그라운드로 돌린다. **한 셸 루프에 여섯 개를 묶으면
10분 한도에 걸린다**(세션 D3 기록). 이어서 `just schoolx-upstream-check` 3/3과
`pnpm test:e2e:smoke workspace-catalog` 5/5 회귀.

- [x] **Step 7: 커밋한다**

```bash
git add desktop/tests desktop/playwright.config.ts docs/schoolx-2 CONTRIBUTING.md
git commit -s -m "docs(schoolx-2): 세션 G — 자체 호스팅 온보딩 결과 기록"
```
