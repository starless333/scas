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
let mathJaxReady;
const MATHJAX_SOURCES = [
  "https://cdn.jsdelivr.net/npm/mathjax@4.0.0/tex-chtml.js",
  "https://unpkg.com/mathjax@4.0.0/tex-chtml.js",
];

function loadMathJaxScript(source) {
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    const timeout = setTimeout(() => {
      script.remove();
      reject(new Error(`MathJax timed out from ${new URL(source).host}`));
    }, 7000);
    script.src = source;
    script.async = true;
    script.onload = () => {
      clearTimeout(timeout);
      resolve();
    };
    script.onerror = () => {
      clearTimeout(timeout);
      script.remove();
      reject(new Error(`MathJax could not load from ${new URL(source).host}`));
    };
    document.head.append(script);
  });
}

function ensureMathJax() {
  if (window.MathJax?.typesetPromise && window.MathJax?.tex2chtmlPromise) return Promise.resolve();
  // Share one lazy load between preview, print, and REPL. A second pinned
  // origin avoids turning one blocked CDN into a blank mathematical surface.
  mathJaxReady ??= (async () => {
    let lastError;
    for (const source of MATHJAX_SOURCES) {
      try {
        await loadMathJaxScript(source);
        await window.MathJax?.startup?.promise;
        if (!window.MathJax?.typesetPromise || !window.MathJax?.tex2chtmlPromise) {
          throw new Error("MathJax is unavailable");
        }
        return;
      } catch (error) {
        lastError = error;
      }
    }
    throw lastError ?? new Error("MathJax could not load");
  })().catch(error => {
    mathJaxReady = undefined;
    throw error;
  });
  return mathJaxReady;
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
    if (version !== previewVersion) return;
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

const replHistory = document.getElementById("repl-history");
const replEntryPreview = document.getElementById("repl-entry-preview");
let replPreviewSource;
let replSequence = 0;
let replTypesetQueue = Promise.resolve();
const replPlotUrls = new Set();

function typesetRepl(element, after = () => {}) {
  replTypesetQueue = replTypesetQueue.catch(() => {}).then(async () => {
    if (!element.isConnected) return;
    await ensureMathJax();
    if (!element.isConnected) return;
    const targets = element.matches("[data-latex]")
      ? [element]
      : [...element.querySelectorAll("[data-latex]")];
    for (const target of targets) {
      const rendered = await window.MathJax.tex2chtmlPromise(target.dataset.latex, { display: true });
      if (!target.isConnected) return;
      target.replaceChildren(rendered);
      target.dataset.typeset = "true";
    }
    element.classList.add("math-ready");
    after();
  }).catch(error => {
    if (!element.isConnected) return;
    const retry = document.createElement("button");
    retry.type = "button";
    retry.className = "repl-typeset-retry";
    retry.textContent = "Math rendering unavailable. Retry";
    retry.title = String(error);
    retry.onclick = () => { retry.remove(); typesetRepl(element, after); };
    element.append(retry);
  });
}

function replMath(latex, className) {
  const math = document.createElement("div");
  math.className = className;
  math.dataset.latex = latex;
  math.textContent = latex;
  return math;
}

window.scasReplResult = (json) => {
  const result = JSON.parse(json);
  // Results are generated while the REPL is active. Reveal the surface here
  // as well as during layout so a response can never land in a hidden panel.
  replHistory.hidden = false;
  if (result.clear || (result.input?.trim() === ":reset" && !result.error)) {
    window.MathJax?.typesetClear?.([replHistory]);
    replHistory.replaceChildren();
    for (const url of replPlotUrls) URL.revokeObjectURL(url);
    replPlotUrls.clear();
    replSequence = 0;
    const empty = document.createElement("p");
    empty.className = "repl-empty";
    empty.textContent = result.input?.trim() === ":reset" ? "Session reset. Enter a new calculation below." : "History cleared. Your definitions are still available.";
    replHistory.append(empty);
    return;
  }
  const follow = replHistory.scrollHeight - replHistory.scrollTop - replHistory.clientHeight < 64;
  replHistory.querySelector(".repl-empty")?.remove();
  const entry = document.createElement("article");
  entry.className = "repl-calculation";
  entry.setAttribute("aria-label", `Calculation ${++replSequence}`);
  const source = document.createElement("div");
  source.className = "repl-source";
  const number = document.createElement("span");
  number.className = "repl-number";
  number.textContent = String(replSequence);
  const code = document.createElement("code");
  code.textContent = result.input ?? "";
  source.append(number, code);
  entry.append(source);
  if (result.input_latex) entry.append(replMath(result.input_latex, "repl-input-math"));
  if (result.result_latex) entry.append(replMath(result.result_latex, "repl-result-math"));
  if ((result.printed ?? "").trim()) {
    const printed = document.createElement("pre");
    printed.textContent = result.printed.trimEnd();
    entry.append(printed);
  }
  if ((result.output ?? "").trim()) {
    const output = document.createElement("pre");
    output.className = "repl-result-text";
    if (result.result_latex) output.classList.add("repl-math-fallback");
    output.textContent = result.output.trimEnd();
    entry.append(output);
  }
  if (result.plot) {
    const plot = result.plot;
    if (plot.vector && plot.svg) {
      // An image document keeps SVG content isolated from the IDE's DOM.
      const url = URL.createObjectURL(new Blob([plot.svg], {type:"image/svg+xml"}));
      replPlotUrls.add(url);
      const image = document.createElement("img");
      image.src = url;
      image.alt = `Plot of ${result.input}`;
      image.className = "repl-plot";
      entry.append(image);
    } else if (plot.ascii) {
      const ascii = document.createElement("pre");
      ascii.textContent = plot.ascii;
      entry.append(ascii);
    }
    if (plot.svg) {
      const save = document.createElement("button");
      save.type = "button";
      save.textContent = "Download SVG";
      save.onclick = () => window.scasDownload(plot.download_name || "scas-plot.svg", plot.svg, "image/svg+xml");
      entry.append(save);
    }
  }
  if (result.error) {
    const error = document.createElement("pre");
    error.className = "repl-error";
    error.setAttribute("role", "alert");
    error.textContent = result.error;
    entry.append(error);
  }
  replHistory.append(entry);
  const scroll = () => { if (follow) replHistory.scrollTop = replHistory.scrollHeight; };
  scroll();
  if (result.input_latex || result.result_latex) typesetRepl(entry, scroll);
};

window.scasShowReplSurface = (surface, content, x, y, width, height, pixelsPerPoint) => {
  const element = surface === "history" ? replHistory : replEntryPreview;
  const bounds = canvas.getBoundingClientRect();
  const scale = pixelsPerPoint / (canvas.width / bounds.width);
  for (const [property, value] of Object.entries({left: bounds.left + x * scale, top: bounds.top + y * scale, width: width * scale, height: height * scale})) {
    const css = `${value}px`;
    if (element.style[property] !== css) element.style[property] = css;
  }
  element.hidden = false;
  if (surface !== "entry" || content === replPreviewSource) return;
  replPreviewSource = content;
  window.MathJax?.typesetClear?.([element]);
  const { latex, input } = JSON.parse(content);
  if (latex) {
    const math = replMath(latex, "repl-live-math");
    element.replaceChildren(math);
    typesetRepl(math);
  } else {
    const hint = document.createElement("span");
    hint.className = "repl-hint";
    hint.textContent = input.trimStart().startsWith(":") ? "Command ready — press Enter to run" : input.trim() ? "Keep typing to complete the expression…" : "Your expression will typeset here as you work";
    element.replaceChildren(hint);
  }
};
window.scasHideRepl = () => {
  replHistory.hidden = true;
  replEntryPreview.hidden = true;
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
