import init, { worker_evaluate } from "./pkg/scas.js";

const ready = init();
self.onmessage = async (event) => {
  try {
    await ready;
    self.postMessage(worker_evaluate(event.data));
  } catch (error) {
    let generation = 0;
    try { generation = JSON.parse(event.data).generation ?? 0; } catch { /* Invalid request. */ }
    self.postMessage(JSON.stringify({ generation, error: String(error) }));
  }
};
