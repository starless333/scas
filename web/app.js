let handle;
// Browser integrations can use the same offline language service as the IDE.
window.scasCreateLspClient = async (onNotification) => {
  const { ScasLspClient } = await import("./lsp.js");
  return new ScasLspClient(onNotification);
};
const loading = document.getElementById("loading");
const loadingMessage = document.getElementById("loading-message");
const loadingDetail = document.getElementById("loading-detail");
const loadingRetry = document.getElementById("loading-retry");
const canvas = document.getElementById("scas-canvas");
const preview = document.getElementById("math-preview");
const previewContent = document.getElementById("math-preview-content");
const previewState = document.getElementById("math-preview-state");
const previewCopy = document.getElementById("math-preview-copy");
const previewZoomOut = document.getElementById("math-preview-zoom-out");
const previewZoomReset = document.getElementById("math-preview-zoom-reset");
const previewZoomIn = document.getElementById("math-preview-zoom-in");
const previewResizer = document.getElementById("math-preview-resizer");
const previewError = document.getElementById("math-preview-error");
const previewSourceCode = document.getElementById("math-preview-source-code");
document.getElementById("math-preview-close")?.addEventListener("click", () => window.scasClosePreview?.());
let previewSource = null;
let previewQueue = Promise.resolve();
async function ensureMathJax() {
  if (window.MathJax?.typesetPromise) return;
  const script = document.querySelector('script[src*="mathjax"]');
  if (window.MathJax?.startup?.promise) await window.MathJax.startup.promise;
  else await new Promise((resolve, reject) => {
    if (!script) return reject(new Error("MathJax script is missing"));
    script.addEventListener("load", resolve, { once: true });
    script.addEventListener("error", reject, { once: true });
  });
  if (!window.MathJax?.typesetPromise) throw new Error("MathJax is unavailable");
}
let previewTex = "";
let previewVersion = 0;
let previewScale = 1;
let previewRight = 0;
let previewWidthPoints = 220;
const PREVIEW_ZOOM_MIN = 0.5;
const PREVIEW_ZOOM_MAX = 16;
const PREVIEW_ZOOM_STEP = 1.25;
let storedPreviewZoom = 1;
try {
  storedPreviewZoom = Number.parseFloat(localStorage.getItem("scas.previewZoom") ?? "1");
} catch { /* Storage may be disabled; zoom still works for this session. */ }
let previewZoom = Number.isFinite(storedPreviewZoom)
  ? Math.min(PREVIEW_ZOOM_MAX, Math.max(PREVIEW_ZOOM_MIN, storedPreviewZoom))
  : 1;

function updatePreviewZoom(nextZoom, anchor) {
  const previousZoom = previewZoom;
  previewZoom = Math.min(PREVIEW_ZOOM_MAX, Math.max(PREVIEW_ZOOM_MIN, nextZoom));
  const panelRect = preview.getBoundingClientRect();
  const localX = anchor ? anchor.clientX - panelRect.left : panelRect.width / 2;
  const localY = anchor ? anchor.clientY - panelRect.top : panelRect.height / 2;
  const contentX = (preview.scrollLeft + localX) / previousZoom;
  const contentY = (preview.scrollTop + localY) / previousZoom;
  previewContent.style.zoom = String(previewZoom);
  previewZoomReset.textContent = `${Math.round(previewZoom * 100)}%`;
  previewZoomOut.disabled = previewZoom <= PREVIEW_ZOOM_MIN;
  previewZoomIn.disabled = previewZoom >= PREVIEW_ZOOM_MAX;
  try {
    localStorage.setItem("scas.previewZoom", String(previewZoom));
  } catch { /* Storage may be disabled; keep the in-memory setting. */ }
  requestAnimationFrame(() => {
    preview.scrollLeft = Math.max(0, contentX * previewZoom - localX);
    preview.scrollTop = Math.max(0, contentY * previewZoom - localY);
  });
}

previewZoomOut?.addEventListener("click", () => updatePreviewZoom(previewZoom / PREVIEW_ZOOM_STEP));
previewZoomReset?.addEventListener("click", () => updatePreviewZoom(1));
previewZoomIn?.addEventListener("click", () => updatePreviewZoom(previewZoom * PREVIEW_ZOOM_STEP));
previewContent?.addEventListener("wheel", (event) => {
  // Trackpad pinches arrive as ctrl+wheel. Leave ordinary two-finger motion
  // alone so it scrolls the document instead of changing its scale.
  if (!event.ctrlKey) return;
  event.preventDefault();
  updatePreviewZoom(previewZoom * Math.exp(-event.deltaY / 300), event);
}, { passive: false });
if (preview && previewContent && previewZoomOut && previewZoomReset && previewZoomIn) {
  updatePreviewZoom(previewZoom);
}

function setPreviewWidth(width) {
  previewWidthPoints = Math.max(220, Math.min(900, width));
  handle?.set_preview_width(previewWidthPoints);
}

previewResizer?.addEventListener("pointerdown", (event) => {
  event.preventDefault();
  previewResizer.setPointerCapture(event.pointerId);
  previewResizer.classList.add("dragging");
});
previewResizer?.addEventListener("pointermove", (event) => {
  if (!previewResizer.hasPointerCapture(event.pointerId)) return;
  setPreviewWidth((previewRight - event.clientX) / previewScale);
});
previewResizer?.addEventListener("pointerup", (event) => {
  if (previewResizer.hasPointerCapture(event.pointerId)) {
    previewResizer.releasePointerCapture(event.pointerId);
  }
  previewResizer.classList.remove("dragging");
});
function cancelPreviewResize(event) {
  if (event?.pointerId != null && previewResizer?.hasPointerCapture(event.pointerId)) {
    previewResizer.releasePointerCapture(event.pointerId);
  }
  previewResizer?.classList.remove("dragging");
}
previewResizer?.addEventListener("pointercancel", cancelPreviewResize);
previewResizer?.addEventListener("lostpointercapture", cancelPreviewResize);
window.addEventListener("blur", cancelPreviewResize);
previewResizer?.addEventListener("dblclick", () => handle?.reset_preview_width());
previewResizer?.addEventListener("keydown", (event) => {
  if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
  event.preventDefault();
  setPreviewWidth(previewWidthPoints + (event.key === "ArrowLeft" ? 20 : -20));
});

previewCopy?.addEventListener("click", async () => {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(previewTex);
    } else {
      const textarea = document.createElement("textarea");
      textarea.value = previewTex;
      textarea.style.position = "fixed";
      textarea.style.opacity = "0";
      document.body.append(textarea);
      textarea.select();
      if (!document.execCommand("copy")) throw new Error("Copy is unavailable");
      textarea.remove();
    }
    previewCopy.textContent = "Copied";
  } catch {
    previewCopy.textContent = "Copy failed";
  }
  setTimeout(() => { previewCopy.textContent = "Copy TeX"; }, 1500);
});

function fillPreview(fragments) {
  window.MathJax?.typesetClear?.([previewContent]);
  const page = document.createElement("div");
  page.className = "preview-page";
  if (!fragments.length) {
    const empty = document.createElement("p");
    empty.className = "preview-empty";
    empty.textContent = "Add a render line, for example: \\[ (x + 1) / (2x^2) \\]";
    page.append(empty);
  }
  for (const fragment of fragments) {
    if (fragment.kind === "prose") {
      for (const line of fragment.latex.split("\n")) {
        if (line.includes("\\hrule")) {
          page.append(document.createElement("hr"));
          continue;
        }
        if (line.includes("\\begin{center}")) {
          const centered = line.replaceAll("\\begin{center}", "").replaceAll("\\end{center}", "")
            .split(/\\\\\[[^\]]*\]/);
          centered.forEach((part, index) => {
            const element = document.createElement(index === 0 ? "h2" : "p");
            element.className = "preview-centered";
            const clean = part.replace(/\{\\(?:Large|large)\\bfseries\s*([^{}]*)\}/g, "$1")
              .replace(/\{\\(?:Large|large)\s*([^{}]*)\}/g, "$1");
            appendProse(element, clean);
            page.append(element);
          });
          continue;
        }
        const heading = line.trim().match(/^\\(section|subsection)\*\{(.*)\}$/);
        const element = document.createElement(heading ? heading[1] === "section" ? "h2" : "h3" : "p");
        if (!heading) element.className = "preview-prose";
        appendProse(element, heading ? heading[2] : line);
        page.append(element);
      }
    } else {
      const block = document.createElement("div");
      block.className = "preview-math";
      block.title = `Source line ${fragment.span?.line ?? 1}`;
      block.tabIndex = 0;
      block.setAttribute("role", "button");
      block.setAttribute("aria-label", `Go to source line ${fragment.span?.line ?? 1}`);
      block.textContent = fragment.latex;
      const jump = () => handle?.jump_to_source(fragment.span?.line ?? 1, fragment.span?.column ?? 1);
      block.addEventListener("click", jump);
      block.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          jump();
        }
      });
      page.append(block);
    }
  }
  previewContent.replaceChildren(page);
}

function appendProse(element, source) {
  const pattern = /\\(textbf|textit|emph)\{([^{}]*)\}/g;
  let offset = 0;
  for (const match of source.matchAll(pattern)) {
    element.append(document.createTextNode(source.slice(offset, match.index)));
    const styled = document.createElement(match[1] === "textbf" ? "strong" : "em");
    styled.textContent = match[2];
    element.append(styled);
    offset = match.index + match[0].length;
  }
  element.append(document.createTextNode(source.slice(offset)));
}

window.scasShowPreview = (fragmentsJson, tex, error, pending, x, y, width, height, pixelsPerPoint, resizable) => {
  if (previewTex !== tex) {
    previewTex = tex;
    if (previewSourceCode) previewSourceCode.textContent = tex;
  }
  const state = pending ? "UPDATING" : "LIVE";
  if (previewState.textContent !== state) previewState.textContent = state;
  if (previewError) {
    if (previewError.hidden !== !error) previewError.hidden = !error;
    const message = error ? `Editing: ${error} (showing last good preview)` : "";
    if (previewError.textContent !== message) previewError.textContent = message;
  }
  const bounds = canvas.getBoundingClientRect();
  previewScale = pixelsPerPoint / (canvas.width / bounds.width);
  const left = bounds.left + x * previewScale;
  const top = bounds.top + y * previewScale;
  const cssWidth = width * previewScale;
  const cssHeight = height * previewScale;
  previewRight = left + cssWidth;
  previewWidthPoints = width;
  if (preview.style.left !== `${left}px`) preview.style.left = `${left}px`;
  if (preview.style.top !== `${top}px`) preview.style.top = `${top}px`;
  if (preview.style.width !== `${cssWidth}px`) preview.style.width = `${cssWidth}px`;
  if (preview.style.height !== `${cssHeight}px`) preview.style.height = `${cssHeight}px`;
  if (preview.hidden) preview.hidden = false;
  if (previewResizer.hidden !== !resizable) previewResizer.hidden = !resizable;
  if (resizable) {
    if (previewResizer.style.left !== `${left - 5}px`) previewResizer.style.left = `${left - 5}px`;
    if (previewResizer.style.top !== `${top}px`) previewResizer.style.top = `${top}px`;
    if (previewResizer.style.height !== `${cssHeight}px`) previewResizer.style.height = `${cssHeight}px`;
    const currentWidth = String(Math.round(width));
    if (previewResizer.getAttribute("aria-valuenow") !== currentWidth) {
      previewResizer.setAttribute("aria-valuenow", currentWidth);
    }
  }
  if (fragmentsJson === previewSource) return;
  previewSource = fragmentsJson;
  const version = ++previewVersion;
  const fragments = JSON.parse(fragmentsJson);
  previewQueue = previewQueue.then(async () => {
    if (version !== previewVersion) return;
    fillPreview(fragments);
    await ensureMathJax();
    await window.MathJax.typesetPromise([previewContent]);
  }).catch((error) => {
    console.error("Math preview failed", error);
    const message = document.createElement("p");
    message.className = "preview-error";
    message.textContent = "Mathematics could not be typeset. Check your connection and reload.";
    previewContent.prepend(message);
  });
};

window.scasHidePreview = () => {
  if (!preview.hidden) preview.hidden = true;
  if (!previewResizer.hidden) previewResizer.hidden = true;
};

window.scasCreateWorker = () => new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
// A busy worker cannot service an interrupt message. Notebook and IDE hosts
// must terminate it, discard that generation, and create a fresh worker.
window.scasInterruptWorker = (worker) => worker?.terminate();

window.scasDownload = (name, content, mime) => {
  const url = URL.createObjectURL(new Blob([content], { type: mime }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  setTimeout(() => URL.revokeObjectURL(url), 30_000);
};

window.scasImport = () => {
  const input = document.createElement("input");
  input.type = "file";
  input.accept = ".scas,.csv,.json,text/plain,text/csv,application/json";
  input.multiple = true;
  input.onchange = async () => {
    for (const file of input.files ?? []) {
      if (!handle) return;
      const content = await file.text();
      handle.import_data_file(file.name, content);
      if (file.name.toLowerCase().endsWith(".scas")) handle.load_source_named(file.name, content);
    }
  };
  input.click();
};

window.scasImportTheme = () => {
  const input = document.createElement("input");
  input.type = "file";
  input.accept = ".toml,text/plain";
  input.onchange = async () => {
    const file = input.files?.[0];
    if (file && handle) handle.import_theme_source(file.name, await file.text());
  };
  input.click();
};

window.scasImportFont = () => {
  const input = document.createElement("input");
  input.type = "file";
  input.accept = ".ttf,.otf,font/ttf,font/otf";
  input.onchange = async () => {
    const file = input.files?.[0];
    if (file && handle) handle.import_font(file.name, new Uint8Array(await file.arrayBuffer()));
  };
  input.click();
};

window.scasSetPreviewTheme = (panel, editor, text, muted, accent, selection) => {
  const style = document.documentElement.style;
  style.setProperty("--scas-panel", panel);
  style.setProperty("--scas-editor", editor);
  style.setProperty("--scas-text", text);
  style.setProperty("--scas-muted", muted);
  style.setProperty("--scas-accent", accent);
  style.setProperty("--scas-selection", selection);
  document.documentElement.style.colorScheme = getComputedStyle(document.documentElement)
    .getPropertyValue("--scas-editor").trim() === "#ffffff" ? "light" : "dark";
};

window.scasPrintDocument = async (fragmentsJson, tex) => {
  const wasHidden = preview.hidden;
  preview.hidden = false;
  try {
    await previewQueue;
    previewSource = fragmentsJson;
    fillPreview(JSON.parse(fragmentsJson));
    await ensureMathJax();
    await window.MathJax.typesetPromise([previewContent]);
    previewTex = tex;
    if (previewSourceCode) previewSourceCode.textContent = tex;
    const restore = () => { if (wasHidden) preview.hidden = true; };
    window.addEventListener("afterprint", restore, { once: true });
    window.print();
  } catch (error) {
    if (wasHidden) preview.hidden = true;
    console.error("PDF export failed", error);
    window.alert(`Could not prepare the PDF preview: ${error}`);
  }
};

window.scasPromptInput = (message) => window.prompt(message);

document.addEventListener("dragover", (event) => event.preventDefault());
document.addEventListener("drop", async (event) => {
  event.preventDefault();
  for (const file of event.dataTransfer?.files ?? []) {
    if (!/\.(scas|csv|json)$/i.test(file.name)) continue;
    const content = await file.text();
    handle?.import_data_file(file.name, content);
    if (file.name.toLowerCase().endsWith(".scas")) handle?.load_source_named(file.name, content);
  }
});

window.addEventListener("beforeunload", (event) => {
  if (!handle?.is_dirty()) return;
  event.preventDefault();
  event.returnValue = "";
});

function showStartupError(error) {
  const detail = error instanceof Error ? error.message : String(error);
  const missingBundle = /scas\.js|dynamically imported module|module script/i.test(detail);
  loading?.classList.add("error");
  if (loadingMessage) loadingMessage.textContent = "SCAS could not start";
  if (loadingDetail) {
    loadingDetail.textContent = missingBundle
      ? "The browser build is missing or unavailable. Run `bash scripts/build-web.sh`, then restart the web server."
      : detail;
  }
  if (loadingRetry) loadingRetry.hidden = false;
  console.error("SCAS startup failed", error);
}

loadingRetry?.addEventListener("click", () => window.location.reload());

try {
  if (loadingMessage) loadingMessage.textContent = "Loading the browser engine…";
  // Keep this import inside the error boundary. A static import failure stops the
  // entire module before a useful startup error can be shown.
  const { default: init, WebHandle } = await import("./pkg/scas.js");
  await init();
  if (loadingMessage) loadingMessage.textContent = "Opening the mathematical workbench…";
  handle = new WebHandle();
  await handle.start("scas-canvas");
  window.scasClosePreview = () => handle.close_preview();
  loading?.remove();
} catch (error) {
  showStartupError(error);
}
