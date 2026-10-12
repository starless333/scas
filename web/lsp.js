/** JSON-RPC LSP client for the offline SCAS worker. Each client owns its documents. */
export class ScasLspClient {
  constructor(onNotification = () => {}) {
    this.worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
    this.onNotification = onNotification;
    this.pending = new Map();
    this.nextId = 1;
    this.worker.onmessage = ({ data }) => {
      const packet = JSON.parse(data);
      for (const notification of packet.notifications ?? []) this.onNotification(notification);
      const response = packet.response;
      if (response?.id == null) return;
      const pending = this.pending.get(response.id);
      if (!pending) return;
      this.pending.delete(response.id);
      if (response.error) pending.reject(new Error(response.error.message));
      else pending.resolve(response.result);
    };
    this.worker.onerror = (event) => {
      for (const pending of this.pending.values()) pending.reject(new Error(event.message));
      this.pending.clear();
    };
  }

  send(message) {
    this.worker.postMessage(JSON.stringify({ kind: "lsp", message: { jsonrpc: "2.0", ...message } }));
  }

  request(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ id, method, params });
    });
  }

  notify(method, params = {}) {
    this.send({ method, params });
  }

  dispose() {
    this.worker.terminate();
    for (const pending of this.pending.values()) pending.reject(new Error("LSP client closed"));
    this.pending.clear();
  }
}
