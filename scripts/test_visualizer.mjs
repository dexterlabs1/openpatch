import assert from "node:assert/strict";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const previewUrl = "http://127.0.0.1:8765/assets/visualizer.html";
const screenshotDirectory = path.join(repository, "visualizer-test-output");
const browser = await chromium.launch({
  headless: true,
  args: ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
});
const scratch = await mkdtemp(path.join(os.tmpdir(), "openpatch-visualizer-"));

function makeWav(seconds = 3, sampleRate = 48000) {
  const frameCount = Math.floor(seconds * sampleRate);
  const dataBytes = frameCount * 2;
  const wav = Buffer.alloc(44 + dataBytes);
  wav.write("RIFF", 0);
  wav.writeUInt32LE(36 + dataBytes, 4);
  wav.write("WAVE", 8);
  wav.write("fmt ", 12);
  wav.writeUInt32LE(16, 16);
  wav.writeUInt16LE(1, 20);
  wav.writeUInt16LE(1, 22);
  wav.writeUInt32LE(sampleRate, 24);
  wav.writeUInt32LE(sampleRate * 2, 28);
  wav.writeUInt16LE(2, 32);
  wav.writeUInt16LE(16, 34);
  wav.write("data", 36);
  wav.writeUInt32LE(dataBytes, 40);
  for (let frame = 0; frame < frameCount; frame++) {
    const sample = Math.round(Math.sin(2 * Math.PI * 440 * frame / sampleRate) * 6000);
    wav.writeInt16LE(sample, 44 + frame * 2);
  }
  return wav;
}

let page;
try {
  await mkdir(screenshotDirectory, { recursive: true });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, deviceScaleFactor: 1 });
  page = await context.newPage();
  const pageErrors = [];
  const offOriginRequests = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  page.on("request", (request) => {
    const requestUrl = new URL(request.url());
    if (requestUrl.origin !== "http://127.0.0.1:8765" && requestUrl.protocol !== "blob:") {
      offOriginRequests.push(request.url());
    }
  });

  await page.goto(previewUrl, { waitUntil: "networkidle" });
  await page.locator("#rendererLabel").waitFor({ state: "visible" });
  await page.waitForFunction(() => document.querySelector("#rendererLabel")?.textContent.includes("ready"));
  assert.equal(await page.locator("#canvasError").isVisible(), false, "WebGL fallback error appeared");
  assert.equal(await page.locator("#shaderStatus").textContent(), "Built-in shader active");
  await page.waitForTimeout(350);
  await page.screenshot({ path: path.join(screenshotDirectory, "visualizer-desktop.png"), fullPage: true });

  const spectrum = page.locator('.mode-button[data-mode="1"]');
  await spectrum.click();
  assert.equal(await spectrum.getAttribute("aria-pressed"), "true");
  assert.equal(await page.locator("#modeStageLabel").textContent(), "Spectrum");
  const oscilloscope = page.locator('.mode-button[data-mode="2"]');
  await oscilloscope.click();
  assert.equal(await oscilloscope.getAttribute("aria-pressed"), "true");
  assert.equal(await page.locator("#modeStageLabel").textContent(), "Oscilloscope");

  await page.locator("#shaderDetails summary").click();
  const source = page.locator("#shaderSource");
  const originalShader = await source.inputValue();
  await source.fill(`#version 300 es
precision highp float;
out vec4 outColor;
void main() { broken shader; }`);
  await page.locator("#applyShader").click();
  await page.waitForFunction(() => document.querySelector("#shaderStatus")?.textContent.includes("Compile failed"));
  assert.match(await page.locator("#shaderStatus").textContent(), /previous shader still active/);
  await source.fill(originalShader);
  await page.locator("#applyShader").click();
  await page.waitForFunction(() => document.querySelector("#shaderStatus")?.textContent.includes("compiled"));
  assert.equal(await page.locator("#shaderStatus").evaluate((element) => element.classList.contains("error")), false);

  const wavPath = path.join(scratch, "tone.wav");
  await writeFile(wavPath, makeWav());
  await page.locator("#audioFile").setInputFiles(wavPath);
  await page.waitForFunction(() => document.querySelector("#duration")?.textContent === "0:03");
  assert.equal(await page.locator("#playButton").isEnabled(), true);
  await page.locator("#volume").evaluate((element) => {
    element.value = "0";
    element.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await page.locator("#playButton").click();
  await page.waitForFunction(() => document.querySelector("#signalLabel")?.textContent === "SIGNAL ACTIVE");
  await page.waitForFunction(() => document.querySelector("#audio")?.currentTime > 0.15, null, { timeout: 5000 });
  await page.locator("#playButton").click();
  await page.waitForFunction(() => document.querySelector("#signalLabel")?.textContent === "SIGNAL READY");

  await page.setViewportSize({ width: 390, height: 844 });
  await page.waitForTimeout(150);
  const mobileWidth = await page.evaluate(() => ({
    viewport: document.documentElement.clientWidth,
    content: document.documentElement.scrollWidth,
  }));
  assert.ok(mobileWidth.content <= mobileWidth.viewport, `mobile layout overflows horizontally: ${JSON.stringify(mobileWidth)}`);
  await page.screenshot({ path: path.join(screenshotDirectory, "visualizer-mobile.png"), fullPage: true });

  assert.deepEqual(offOriginRequests, [], `preview made external requests: ${offOriginRequests.join(", ")}`);
  assert.deepEqual(pageErrors, [], `browser reported page errors: ${pageErrors.join("; ")}`);
  console.log("Visualizer browser smoke passed; screenshots saved to visualizer-test-output/.");
  await context.close();
} finally {
  await browser.close();
  await rm(scratch, { recursive: true, force: true });
}
