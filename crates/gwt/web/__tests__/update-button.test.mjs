// SPEC-2041 Phase 14 — unified GUI update CTA.
//
// The update notification must be one actionable bottom-right CTA, not a
// transient toast plus a separate persistent button. These tests exercise the
// CTA controller with DOM-like elements so regressions are caught by behavior,
// not source-string proximity.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { createUpdateCtaController } from "../update-cta.js";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const indexHtml = readFileSync(resolve(here, "../index.html"), "utf8");

test("update_state renders one reusable update CTA", () => {
  const fixture = createFixture();
  const controller = createUpdateCtaController(fixture.options);

  controller.handleUpdateState({
    state: "available",
    current: "9.22.0",
    latest: "9.23.0",
  });
  controller.handleUpdateState({
    state: "available",
    current: "9.22.0",
    latest: "9.23.0",
  });

  assert.equal(fixture.document.querySelectorAll("#update-cta").length, 1);
  const cta = fixture.document.getElementById("update-cta");
  assert.equal(cta.textContent, "Update available: v9.23.0");
  assert.equal(cta.dataset.status, "available");
  assert.equal(fixture.versionUpdates.length, 2);
});

test("update CTA click cancel leaves it available and does not send apply_update", () => {
  const fixture = createFixture({ confirmResult: false });
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.23.0");

  fixture.document.getElementById("update-cta").click();

  assert.equal(fixture.confirmCalls.length, 1);
  assert.deepEqual(fixture.sent, []);
  assert.equal(fixture.document.getElementById("update-cta").dataset.status, "available");
});

test("update CTA click approve sends apply_update and shows applying state", () => {
  const fixture = createFixture({ confirmResult: true });
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.23.0");

  fixture.document.getElementById("update-cta").click();

  assert.deepEqual(fixture.sent, [{ kind: "apply_update" }]);
  const cta = fixture.document.getElementById("update-cta");
  assert.equal(cta.dataset.status, "applying");
  assert.equal(cta.disabled, true);
  assert.equal(cta.textContent, "Applying update...");
});

test("duplicate update_state does not reset an applying CTA", () => {
  const fixture = createFixture({ confirmResult: true });
  const controller = createUpdateCtaController(fixture.options);
  controller.handleUpdateState({
    state: "available",
    current: "9.22.0",
    latest: "9.23.0",
  });
  fixture.document.getElementById("update-cta").click();

  controller.handleUpdateState({
    state: "available",
    current: "9.22.0",
    latest: "9.23.0",
  });

  const cta = fixture.document.getElementById("update-cta");
  assert.equal(cta.dataset.status, "applying");
  assert.equal(cta.disabled, true);
  assert.equal(cta.textContent, "Applying update...");
});

test("update_apply_error reuses the same CTA and allows retry", () => {
  const fixture = createFixture({ confirmResult: true });
  const controller = createUpdateCtaController(fixture.options);
  controller.showAvailable("9.23.0");
  fixture.document.getElementById("update-cta").click();

  controller.showError("Failed to start the update.");
  const cta = fixture.document.getElementById("update-cta");

  assert.equal(fixture.document.querySelectorAll("#update-cta").length, 1);
  assert.equal(cta.dataset.status, "error");
  assert.equal(cta.disabled, false);
  assert.match(cta.textContent, /Update failed/);
  assert.match(cta.textContent, /Failed to start the update/);

  cta.click();
  assert.deepEqual(fixture.sent, [{ kind: "apply_update" }, { kind: "apply_update" }]);
});

test("app.js delegates update handling to the unified update CTA controller", () => {
  assert.match(appSource, /createUpdateCtaController/);
  assert.match(appSource, /updateCtaController\.handleUpdateState\(event\)/);
  assert.match(appSource, /updateCtaController\.showError\(/);
});

test("legacy split update toast and button surfaces are removed", () => {
  assert.doesNotMatch(appSource, /showUpdateToast/);
  assert.doesNotMatch(appSource, /showUpdateButton/);
  assert.doesNotMatch(indexHtml, /\.update-toast\b/);
  assert.doesNotMatch(indexHtml, /\.update-button\b/);
});

test("index.html declares a fixed bottom-right unified update CTA style", () => {
  const styleMatch = indexHtml.match(/\.update-cta\s*\{[^}]+\}/);
  assert.ok(styleMatch, "expected .update-cta rule inside <style>");
  assert.match(styleMatch[0], /position:\s*fixed/);
  assert.match(styleMatch[0], /bottom:\s*\d+px/);
  assert.match(styleMatch[0], /right:\s*\d+px/);
  assert.match(indexHtml, /\.update-cta\.is-applying\s*\{/);
  assert.match(indexHtml, /\.update-cta\.is-error\s*\{/);
});

function createFixture({ confirmResult = true } = {}) {
  const document = new FakeDocument();
  const sent = [];
  const confirmCalls = [];
  const versionUpdates = [];
  return {
    document,
    sent,
    confirmCalls,
    versionUpdates,
    options: {
      document,
      send(message) {
        sent.push(message);
      },
      confirmUpdate(version) {
        confirmCalls.push(version);
        return confirmResult;
      },
      setVersionState(current, latest) {
        versionUpdates.push({ current, latest });
      },
    },
  };
}

class FakeDocument {
  constructor() {
    this.body = new FakeElement("body", this);
    this.elementsById = new Map();
  }

  createElement(tagName) {
    return new FakeElement(tagName, this);
  }

  getElementById(id) {
    return this.elementsById.get(id) || null;
  }

  querySelectorAll(selector) {
    if (!selector.startsWith("#")) return [];
    const element = this.getElementById(selector.slice(1));
    return element ? [element] : [];
  }

  registerId(element, id) {
    if (id) {
      this.elementsById.set(id, element);
    }
  }
}

class FakeElement {
  constructor(tagName, ownerDocument) {
    this.tagName = tagName.toUpperCase();
    this.ownerDocument = ownerDocument;
    this.children = [];
    this.dataset = {};
    this.attributes = new Map();
    this._classNames = new Set();
    this.classList = {
      add: (...tokens) => tokens.forEach((token) => this._classNames.add(token)),
      remove: (...tokens) => tokens.forEach((token) => this._classNames.delete(token)),
      toggle: (token, force) => {
        const shouldAdd = force === undefined ? !this._classNames.has(token) : Boolean(force);
        if (shouldAdd) this._classNames.add(token);
        else this._classNames.delete(token);
      },
      contains: (token) => this._classNames.has(token),
    };
    this.textContent = "";
    this.disabled = false;
    this.onclick = null;
  }

  set id(value) {
    this._id = value;
    this.ownerDocument.registerId(this, value);
  }

  get id() {
    return this._id;
  }

  set className(value) {
    this._classNames = new Set(String(value).split(/\s+/).filter(Boolean));
  }

  get className() {
    return Array.from(this._classNames).join(" ");
  }

  setAttribute(name, value) {
    this.attributes.set(name, String(value));
  }

  getAttribute(name) {
    return this.attributes.get(name) || null;
  }

  appendChild(child) {
    this.children.push(child);
    return child;
  }

  click() {
    if (this.onclick) {
      this.onclick();
    }
  }
}
