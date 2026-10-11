import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import vm from "node:vm";
import ts from "typescript";

// Run the same translation module as the app, including a small state-hook
// harness to check notices retained across a locale change.
let state;
const catalog = JSON.parse(readFileSync(new URL("../src/locales.json", import.meta.url), "utf8"));
const context = {
  exports: {}, navigator: { language: "en-US" },
  localStorage: { getItem: () => null, setItem: () => {} },
  document: { documentElement: {} },
  require: name => name === "react" ? {
    useState: initial => {
      if (!state) state = initial();
      return [state, update => { state = update(state); }];
    },
  } : name === "./storePreview" ? { storePreviewLanguage: null } : name === "./locales.json" ? { default: catalog } : context.exports,
};
function run(path, target = context) {
  const source = readFileSync(new URL(path, import.meta.url), "utf8");
  const code = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS } }).outputText;
  vm.runInNewContext(code, target);
}
run("../src/i18n.ts");
const { setLocale, t, localizedDetail, useLocalizedState, detectLocale, translate } = context.exports;
assert.equal(t("日本語", "English"), "English");
const filterContext = { exports: {}, require: () => context.exports };
run("../src/imageFiles.ts", filterContext);
const { supportedImageFilter, supportedImageShortLabel } = filterContext.exports;
assert.equal(supportedImageFilter.name, "Images (common still-image formats)");
let [notice, update] = useLocalizedState(() => t("画像を確認しています…", "Inspecting images…"));
assert.equal(notice, "Inspecting images…");
update(() => t("3 枚を読み込みました", "Loaded 3 images"));
setLocale("ja");
assert.equal(useLocalizedState()[0], "3 枚を読み込みました");
assert.equal(supportedImageFilter.name, "画像（主要な静止画形式）");
assert.match(supportedImageShortLabel(), /などの画像/);
assert.equal(localizedDetail("PNG optimized", "Done"), "PNGを最適化しました");
setLocale("en");
assert.equal(useLocalizedState()[0], "Loaded 3 images");
assert.equal(supportedImageFilter.name, "Images (common still-image formats)");
assert.equal(localizedDetail("PNG-8では品質が保てないため、可逆PNGで保存しました", "Done"), "Saved as lossless PNG because PNG-8 could not preserve sufficient quality");
assert.equal(localizedDetail("JPEGをPNGに変換しました", "Done"), "Converted JPEG to PNG");
assert.equal(localizedDetail("HEICを読み込めません: decoder failure", "Failed"), "Could not read HEIC: decoder failure");
assert.equal(localizedDetail("画像を開けません: disk full", "Failed"), "Could not open image: disk full");
assert.equal(localizedDetail("未知のエラー", "Failed"), "Failed");

// Guard every app-owned backend error against silently collapsing to a generic
// English message. Dynamic format-name errors are tested separately above.
for (const file of ["lib.rs", "atlas.rs", "texture_tools.rs"]) {
  const source = readFileSync(new URL(`../src-tauri/src/${file}`, import.meta.url), "utf8");
  for (const match of source.matchAll(/(?:Err\(|ok_or\(|ok_or_else\(\|\| )"([^"\n]*[ぁ-んァ-ヶ一-龯][^"\n]*)"/gu)) {
    assert.notEqual(localizedDetail(match[1], "UNTRANSLATED"), "UNTRANSLATED", `${file}: ${match[1]}`);
  }
}
console.log("Translation tests passed: live notices, dialog filters, processing results and backend errors.");

assert.equal(detectLocale("zh-Hans-CN"), "zh-CN");
assert.equal(detectLocale("de-AT"), "de");
assert.equal(detectLocale("fr-FR"), "en");
for (const [lang, button, result, read] of [
  ["zh-CN", "图像优化", "已将 JPEG 转换为 PNG", "无法读取 HEIC: decoder failure"],
  ["de", "Bildoptimierung", "JPEG in PNG konvertiert", "HEIC konnte nicht gelesen werden: decoder failure"],
]) {
  setLocale(lang);
  assert.equal(t("画像最適化", "Image Optimization"), button);
  assert.equal(localizedDetail("JPEGをPNGに変換しました", "Done"), result);
  assert.equal(localizedDetail("HEICを読み込めません: decoder failure", "Failed"), read);
  const failureTail = translate(" (2 failed)", { key: " ({0} failed)", values: [2] });
  const summary = translate("Loaded 3 images (2 failed)", { key: "Loaded {0} image{1}{2}", values: [3, "s", failureTail] });
  assert.ok(summary.includes("3") && summary.includes("2"));
  assert.ok(!summary.includes("failed"), "Nested failure counts must also be translated");
  assert.notEqual(supportedImageFilter.name, "Images (common still-image formats)");
  const name = "日本語名-{1}.png";
  assert.ok(translate(`Preview of ${name}`, { key: "Preview of {0}", values: [name] }).includes(name), "File names must remain unmodified, even if they contain placeholders");
}
setLocale("de");
assert.equal(localizedDetail("请选择图像", "Processing failed"), "Ein Bild wählen");

// Translation coverage is checked from the source AST, not just the screens
// visible in a screenshot. New labels/messages must add both translations.
function requireEntry(key) {
  assert.ok(catalog[key], `Missing translation: ${key}`);
  assert.equal(catalog[key].length, 2, key);
  assert.ok(catalog[key].every(value => value.length > 0), key);
  const expected = [...key.matchAll(/\{(\d+)\}/g)].map(match => match[1]);
  for (const value of catalog[key]) for (const match of value.matchAll(/\{(\d+)\}/g)) {
    assert.ok(expected.includes(match[1]), `Invalid placeholder: ${key}`);
  }
}
for (const file of readdirSync(new URL("../src/", import.meta.url)).filter(name => /\.tsx?$/.test(name))) {
  const source = readFileSync(new URL(`../src/${file}`, import.meta.url), "utf8");
  const tree = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, file.endsWith("tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  function visit(node) {
    if (ts.isCallExpression(node) && node.expression.getText(tree) === "t") {
      const english = node.arguments[1];
      if (ts.isStringLiteral(english)) requireEntry(english.text);
      else if (ts.isTemplateExpression(english)) {
        let key = english.head.text;
        english.templateSpans.forEach((span, index) => { key += `{${index}}${span.literal.text}`; });
        requireEntry(key);
        assert.equal(node.arguments.length, 3, `${file}: parameterized translations need a template key`);
      }
    }
    if (ts.isCallExpression(node) && node.expression.getText(tree) === "localizedDetail" && ts.isStringLiteral(node.arguments[1])) requireEntry(node.arguments[1].text);
    if (file === "HelpPage.tsx" && ts.isStringLiteral(node) && /[A-Za-z].*[ .]/.test(node.text) && !node.text.startsWith(".") && !/[ぁ-んァ-ヶ一-龯]/u.test(node.text)) {
      // English instructions belong to paired arrays; exclude DOM attribute values.
      if (ts.isArrayLiteralExpression(node.parent)) requireEntry(node.text);
    }
    ts.forEachChild(node, visit);
  }
  visit(tree);
}
const nativeSource = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
for (const match of nativeSource.matchAll(/text\("[^"\n]+", "([^"\n]+)"\)/g)) requireEntry(match[1]);
console.log("Four-language tests passed: locale detection, catalog coverage, template parameters, filenames and native menu labels.");
