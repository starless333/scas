import init, { WebHandle } from "./pkg/scas.js";

let handle;
const loading = document.getElementById("loading");
const loadingMessage = document.getElementById("loading-message");
const canvas = document.getElementById("scas-canvas");
const preview = document.getElementById("math-preview");
const previewContent = document.getElementById("math-preview-content");
const previewState = document.getElementById("math-preview-state");
const previewCopy = document.getElementById("math-preview-copy");
document.getElementById("math-preview-close").addEventListener("click", () => window.scasClosePreview?.());
let previewSource = null;
let previewQueue = Promise.resolve();
let previewTex = "";
let previewVersion = 0;
previewCopy.addEventListener("click", async () => {
  await navigator.clipboard.writeText(previewTex);
  previewCopy.textContent = "Copied";
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
      block.textContent = fragment.latex;
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

window.scasShowPreview = (fragmentsJson, tex, pending, x, y, width, height, pixelsPerPoint) => {
  previewTex = tex;
  previewState.textContent = pending ? "UPDATING" : "LIVE";
  const bounds = canvas.getBoundingClientRect();
  const scale = pixelsPerPoint / (canvas.width / bounds.width);
  preview.style.left = `${bounds.left + x * scale}px`;
  preview.style.top = `${bounds.top + y * scale}px`;
  preview.style.width = `${width * scale}px`;
  preview.style.height = `${height * scale}px`;
  preview.hidden = false;
  if (fragmentsJson === previewSource) return;
  previewSource = fragmentsJson;
  const version = ++previewVersion;
  const fragments = JSON.parse(fragmentsJson);
  previewQueue = previewQueue.then(async () => {
    if (version !== previewVersion) return;
    fillPreview(fragments);
    if (!window.MathJax?.typesetPromise) {
      const script = document.querySelector('script[src*="mathjax"]');
      if (window.MathJax?.startup?.promise) await window.MathJax.startup.promise;
      else await new Promise((resolve, reject) => {
        if (!script) return reject(new Error("MathJax script is missing"));
        script.addEventListener("load", resolve, { once: true });
        script.addEventListener("error", reject, { once: true });
      });
    }
    await window.MathJax.typesetPromise([previewContent]);
  }).catch((error) => {
    console.error("Math preview failed", error);
    const message = document.createElement("p");
    message.className = "preview-error";
    message.textContent = "Mathematics could not be typeset. Check your connection and reload.";
    previewContent.prepend(message);
  });
};

window.scasHidePreview = () => { preview.hidden = true; };

window.scasCreateWorker = () => new Worker(new URL("./worker.js", import.meta.url), { type: "module" });

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
  input.accept = ".scas,text/plain";
  input.onchange = async () => {
    const file = input.files?.[0];
    if (file && handle) handle.load_source(await file.text());
  };
  input.click();
};

try {
  await init();
  handle = new WebHandle();
  await handle.start("scas-canvas");
  window.scasClosePreview = () => handle.close_preview();
  loading.remove();
} catch (error) {
  loading.classList.add("error");
  loadingMessage.textContent = `SCAS could not start: ${error}`;
  console.error(error);
}
