import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
const catalog = JSON.parse(readFileSync(new URL("../src/locales.json", import.meta.url), "utf8"));
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ channel: "msedge", headless: true, args: ["--enable-unsafe-swiftshader"] });
try {
  const page = await browser.newPage({ viewport: { width: 1180, height: 700 } });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.addInitScript(() => {
    localStorage.setItem("be-asset-optimizer-language-v1", "en");
    const callbacks = new Map(), listeners = new Map(); let id = 0;
    window.testFilters = [];
    window.emitMenu = action => {
      const callback = callbacks.get(listeners.get("app-menu-action"));
      callback({ event: "app-menu-action", id: 1, payload: action });
    };
    window.emitProgress = (event, payload) => callbacks.get(listeners.get(event))({ event, id: 1, payload });
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
      transformCallback: callback => { callbacks.set(++id, callback); return id; },
      unregisterCallback: () => {}, convertFileSrc: path => path,
      invoke: async (cmd, args) => {
        if (cmd === "plugin:event|listen") { listeners.set(args.event, args.handler); return ++id; }
        if (cmd === "plugin:dialog|open") {
          window.testFilters.push(args.options.filters?.[0]?.name);
          return args.options.multiple ? ["demo/albedo.png"] : "demo/albedo.png";
        }
        if (cmd === "inspect_files") return { files: args.paths.map(path => ({ path, name: "albedo.png", originalBytes: 5000, width: 512, height: 512 })), failures: [] };
        if (cmd === "create_preview") {
          const canvas = document.createElement("canvas"); canvas.width = canvas.height = 4;
          return canvas.toDataURL();
        }
        if (cmd === "pack_channels") throw "画像が大きすぎます（最大64メガピクセル）";
        return null;
      },
    };
  });
  await page.goto(process.env.HELP_CAPTURE_URL || "http://127.0.0.1:1420");
  const panel = page.locator(".tool-panel:not([hidden])");
  await panel.getByRole("button", { name: "Add Files", exact: true }).click();
  await panel.getByText("Loaded 1 image", { exact: true }).waitFor();
  await page.evaluate(() => window.emitProgress("job-progress", { path: "demo/albedo.png", status: "done", message: "PNG-8では品質が保てないため、可逆PNGで保存しました" }));
  await panel.getByText(/Saved as lossless PNG/).waitFor();
  await page.evaluate(() => window.emitMenu("language-ja"));
  await panel.getByText("1 枚を読み込みました", { exact: true }).waitFor();
  await panel.getByText("PNG-8では品質が保てないため、可逆PNGで保存しました", { exact: true }).waitFor();
  assert.equal(await panel.locator(".job-row").count(), 1, "Switching language must retain the queue");
  await panel.getByRole("button", { name: "ファイルを追加", exact: true }).click();
  assert.deepEqual(await page.evaluate(() => window.testFilters), ["Images (common still-image formats)", "画像（主要な静止画形式）"]);
  for (const [tab, title] of [["normal", "ノーマルマップ設定"], ["atlas", "アトラス設定"], ["preview", "3Dプレビュー"], ["material", "PBRプレビュー"], ["channels", "チャンネル設定"], ["adjust", "ノーマル調整"]]) {
    await page.evaluate(tab => window.emitMenu(`tab-${tab}`), tab);
    await panel.getByText(title, { exact: true }).first().waitFor();
  }
  await page.evaluate(() => window.emitMenu("tab-channels"));
  await panel.getByRole("button", { name: "ORM", exact: true }).click();
  await panel.getByText("G · ラフネス", { exact: true }).waitFor();
  await panel.locator(".texture-input").first().click();
  await panel.getByRole("button", { name: "保存", exact: true }).click();
  await panel.getByText("画像が大きすぎます（最大64メガピクセル）", { exact: true }).waitFor();
  await page.evaluate(() => window.emitMenu("language-en"));
  await panel.getByText("G · Roughness", { exact: true }).waitFor();
  await panel.getByText("Image is too large (maximum 64 megapixels)", { exact: true }).waitFor();
  for (const tab of ["optimize", "normal", "atlas", "preview", "material", "channels", "adjust"]) {
    await page.evaluate(tab => window.emitMenu(`tab-${tab}`), tab);
    await page.waitForTimeout(100);
    assert.doesNotMatch(await panel.innerText(), /[ぁ-んァ-ヶ一-龯]/u, `${tab}: English UI must not contain Japanese app labels`);
  }
  await page.keyboard.press("F1");
  await page.getByRole("dialog").waitFor();
  assert.doesNotMatch(await page.getByRole("dialog").innerText(), /[ぁ-んァ-ヶ一-龯]/u);
  await page.keyboard.press("Escape");
  const headings = [
    ["optimize", "Conversion Settings"], ["normal", "Normal Map Settings"],
    ["atlas", "Atlas Settings"], ["preview", "Texture References"],
    ["material", "PBR Preview"], ["channels", "Channel Settings"],
    ["adjust", "Normal Adjustment"],
  ];
  for (const [lang, index] of [["zh-CN", 0], ["de", 1]]) {
    await page.evaluate(lang => window.emitMenu(`language-${lang}`), lang);
    assert.equal(await page.locator("html").getAttribute("lang"), lang);
    for (const [tab, heading] of headings) {
      await page.evaluate(tab => window.emitMenu(`tab-${tab}`), tab);
      await panel.getByRole("heading", { name: catalog[heading][index], exact: true }).waitFor();
      if (tab === "optimize") {
        await panel.getByText(catalog["Saved as lossless PNG because PNG-8 could not preserve sufficient quality"][index], { exact: true }).waitFor();
        assert.equal(await panel.locator(".job-row").count(), 1);
      }
      if (tab === "channels") {
        await panel.getByText(catalog["Image is too large (maximum 64 megapixels)"][index], { exact: true }).waitFor();
        await panel.getByRole("button", { name: catalog["Drop or choose"][index], exact: true }).first().click();
        assert.equal(await page.evaluate(() => window.testFilters.at(-1)), catalog["Images (common still-image formats)"][index]);
      }
      await page.keyboard.press("F1");
      const dialog = page.getByRole("dialog");
      await dialog.getByRole("heading", { name: catalog["Help — Be Asset Optimizer"][index], exact: true }).waitFor();
      assert.ok((await dialog.locator("ol li").first().innerText()).length > 0);
      await dialog.locator("img").evaluate(async image => { if (!image.complete) await new Promise(resolve => { image.onload = image.onerror = resolve; }); });
      assert.ok(await dialog.locator("img").evaluate(image => image.naturalWidth > 0), `${lang}/${tab}: help screenshot must load`);
      await page.keyboard.press("Escape");
    }
  }
  assert.deepEqual(errors, []);
  console.log("UI translation tests passed: all seven tabs in four languages, live menu switching, retained inputs/results/errors, dialog filters and localized help screenshots.");
} finally { await browser.close(); }
