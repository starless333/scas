import init, { WebHandle } from "./pkg/scas.js";

let handle;
const loading = document.getElementById("loading");
const loadingMessage = document.getElementById("loading-message");

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
  loading.remove();
} catch (error) {
  loading.classList.add("error");
  loadingMessage.textContent = `SCAS could not start: ${error}`;
  console.error(error);
}
