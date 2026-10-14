let ready;
self.onmessage = async (event) => {
  try {
    ready ??= import("./pkg/scas.js").then(async ({ default: init, worker_evaluate }) => {
      await init();
      return worker_evaluate;
    });
    const worker_evaluate = await ready;
    self.postMessage(worker_evaluate(event.data));
  } catch (error) {
    let generation = 0;
    let kind = "run";
    let input;
    let lspId;
    try {
      const request = JSON.parse(event.data);
      generation = request.generation ?? 0;
      kind = request.kind?.startsWith("repl") ? "repl" : request.kind ?? "run";
      input = request.input;
      lspId = request.message?.id;
    } catch { /* Invalid request. */ }
    if (kind === "lsp") {
      self.postMessage(JSON.stringify({ kind, response: lspId == null ? null : {
        jsonrpc: "2.0", id: lspId, error: { code: -32603, message: String(error) },
      }, notifications: [] }));
      return;
    }
    self.postMessage(JSON.stringify({ generation, kind, input, error: String(error) }));
  }
};
