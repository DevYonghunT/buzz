import assert from "node:assert/strict";
import test from "node:test";
import { createInstance } from "i18next";

import { createAppI18nInitOptions } from "./config.ts";
import { en } from "./locales/en.ts";
import { ko } from "./locales/ko.ts";
import { APP_I18N_NAMESPACES } from "./resources.ts";

/**
 * i18next keeps the `resources` object by reference and mutates it in place, so
 * an instance that edits its catalog also edits the module-level
 * `appI18nResources` every later instance is built from. Cloning per instance
 * keeps the gap-simulating tests below from leaking into each other.
 */
async function createAppI18nInstance(locale) {
  const options = createAppI18nInitOptions(locale);
  const instance = createInstance();
  await instance.init({
    ...options,
    resources: structuredClone(options.resources),
  });
  return instance;
}

/**
 * Re-register `namespace` for `locale` with `key` deleted, so the catalog has a
 * hole exactly where a translator would leave one. It has to be a remove
 * followed by an add: `addResourceBundle`'s `overwrite` flag overwrites the keys
 * it is given and leaves the rest of the bundle standing, so passing a object
 * with the key omitted does not delete anything.
 */
function dropTranslationKey(instance, locale, namespace, key) {
  const bundle = structuredClone(instance.getResourceBundle(locale, namespace));
  delete bundle[key];
  instance.removeResourceBundle(locale, namespace);
  instance.addResourceBundle(locale, namespace, bundle);
}

function collectLeafKeys(value, prefix = "") {
  return Object.entries(value).flatMap(([key, child]) => {
    const path = prefix ? `${prefix}.${key}` : key;
    return typeof child === "string" ? [path] : collectLeafKeys(child, path);
  });
}

function collectLeafValues(value) {
  return Object.values(value).flatMap((child) =>
    typeof child === "string" ? [child] : collectLeafValues(child),
  );
}

test("English and Korean translation catalogs expose the same keys", () => {
  assert.deepEqual(collectLeafKeys(ko).sort(), collectLeafKeys(en).sort());
});

test("translation catalogs do not contain blank values", () => {
  assert.equal(
    collectLeafValues(en).every((value) => value.trim().length > 0),
    true,
  );
  assert.equal(
    collectLeafValues(ko).every((value) => value.trim().length > 0),
    true,
  );
});

test("top-level resource groups resolve as typed namespaces without changing existing keys", async () => {
  const instance = await createAppI18nInstance("en");

  assert.equal(
    instance.t("app.loading.settingUpCommunity"),
    "Setting up your community…",
  );
  assert.equal(instance.t("settings.sections.appearance"), "Appearance");
  assert.equal(instance.t("appearance.language.ko"), "Korean");
});

test("Korean resolves from the Korean catalog", async () => {
  const instance = await createAppI18nInstance("ko");

  assert.equal(instance.t("appearance.title"), "화면 및 언어");
  assert.equal(instance.t("settings.sidebar.backToApp"), "앱으로 돌아가기");
});

// The catalogs are key-for-key identical today and the parity test above keeps
// them that way, so these holes cannot reach the shipped bundle. What they cover
// is the other half of the contract — that the *configuration* rescues a gap if
// one ever lands. `fallbackLng` is one line, it is the only thing between a
// translator's omission and a raw `appearance.title` rendered at the user, and
// until now it was the one init option no test read.
test("a missing Korean key renders the English string, not the key path", async () => {
  const instance = await createAppI18nInstance("ko");
  dropTranslationKey(instance, "ko", "appearance", "title");

  assert.equal(instance.t("appearance.title"), "Appearance");
});

test("falling back for one key leaves its Korean siblings alone", async () => {
  const instance = await createAppI18nInstance("ko");
  dropTranslationKey(instance, "ko", "appearance", "title");

  assert.equal(
    instance.t("appearance.description"),
    "테마와 인터페이스 언어를 선택하세요.",
  );
  assert.equal(instance.t("settings.sections.appearance"), "화면 및 언어");
});

// Characterization, not an endorsement. A *key* gap falls back; a whole *missing
// namespace* does not, and the difference is invisible from the call site.
//
// `nsSeparator` is ".", so i18next decides whether the prefix of
// "appearance.title" names a namespace by looking at the namespaces loaded for
// the current language. Drop `ko.appearance` and "appearance" stops being a
// namespace, so the whole string is looked up as a key in the default namespace
// and misses. No language fallback can recover it — not even an explicit
// `{ lng: "en" }`, which still returns the key path. Only naming the namespace
// separately, `t("title", { ns: "appearance" })`, resolves.
//
// So nothing at runtime protects a namespace that exists in `en` and not in
// `ko`. What protects it is `ko satisfies TranslationShape<typeof en>` at
// compile time and the key-parity test above. Adding a namespace to
// APP_I18N_NAMESPACES and `en` while forgetting `ko` must stay a build failure —
// if either guard is ever loosened, Korean users get key paths on screen, not
// English.
test("a missing Korean namespace is NOT rescued by fallbackLng", async () => {
  const instance = await createAppI18nInstance("ko");
  instance.removeResourceBundle("ko", "appearance");

  assert.equal(instance.t("appearance.title"), "appearance.title");
  assert.equal(
    instance.t("appearance.title", { lng: "en" }),
    "appearance.title",
  );
  assert.equal(instance.t("title", { ns: "appearance" }), "Appearance");
});

test("every declared namespace is present in both catalogs", async () => {
  for (const namespace of APP_I18N_NAMESPACES) {
    assert.ok(
      Object.hasOwn(en, namespace),
      `en is missing the "${namespace}" namespace`,
    );
    assert.ok(
      Object.hasOwn(ko, namespace),
      `ko is missing the "${namespace}" namespace — its keys would render as key paths, not English`,
    );
  }
});
