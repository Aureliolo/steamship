// Runs axe-core over every page of the built site, once in the light scheme and once in the
// dark, and fails on any violation, naming the page, the rule and each element that breaks it.
//
// The browser is the one the runner image installs, so nothing is downloaded at run time that
// the lock file does not pin.
//
//   node check.mjs <site folder>    with CHROME set to the browser to drive

import { readdir, readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

import puppeteer from "puppeteer-core";

// WCAG 2.2 at levels A and AA, which is what the site is held to, and axe's best practices.
const RULES = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22a", "wcag22aa", "best-practice"];

const site = resolve(process.argv[2] ?? "target/site");
const pages = (await readdir(site)).filter((name) => name.endsWith(".html")).sort();
if (pages.length === 0) {
  console.error(`no pages in ${site}: build the site first`);
  process.exit(1);
}
const axe = await readFile(createRequire(import.meta.url).resolve("axe-core/axe.min.js"), "utf8");

const browser = await puppeteer.launch({ executablePath: process.env.CHROME, headless: true });
let violations = 0;
try {
  for (const scheme of ["light", "dark"]) {
    for (const name of pages) {
      const page = await browser.newPage();
      await page.emulateMediaFeatures([{ name: "prefers-color-scheme", value: scheme }]);
      await page.goto(pathToFileURL(join(site, name)).href, { waitUntil: "load" });
      await page.addScriptTag({ content: axe });
      const found = await page.evaluate(
        (rules) => window.axe.run(document, { runOnly: { type: "tag", values: rules } }),
        RULES,
      );
      for (const violation of found.violations) {
        violations += 1;
        console.log(`${name}, ${scheme}: ${violation.id}: ${violation.help}`);
        for (const node of violation.nodes) {
          console.log(`  ${node.target.join(" ")}: ${node.failureSummary.replaceAll("\n", " ")}`);
        }
      }
      await page.close();
    }
  }
} finally {
  await browser.close();
}

if (violations > 0) {
  console.error(`${violations} accessibility rules broken; each is listed above with its elements`);
  process.exit(1);
}
console.log(`${pages.length} pages, light and dark: no accessibility rule broken`);
