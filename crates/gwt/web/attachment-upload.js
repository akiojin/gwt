let attachmentUploadTokenPromise = null;

async function attachmentUploadToken() {
  if (!attachmentUploadTokenPromise) {
    attachmentUploadTokenPromise = fetch("/internal/attachment-upload-token", {
      credentials: "same-origin",
    }).then(async (response) => {
      if (!response.ok) {
        throw new Error(`attachment upload token failed: ${response.status}`);
      }
      const payload = await response.json();
      if (!payload?.token) {
        throw new Error("attachment upload token missing");
      }
      return payload.token;
    }).catch((error) => {
      // A transient failure must not poison every later user paste/drop.
      attachmentUploadTokenPromise = null;
      throw error;
    });
  }
  return attachmentUploadTokenPromise;
}

export function uploadAttachmentFile(file, { onProgress, signal } = {}) {
  if (typeof window.__gwtAttachmentUploader === "function") {
    return window.__gwtAttachmentUploader({ file, onProgress, signal });
  }
  return new Promise((resolve, reject) => {
    void attachmentUploadToken()
      .then((token) => {
        if (signal?.aborted) {
          reject(new Error("upload aborted"));
          return;
        }
        const params = new URLSearchParams({
          filename: file.name || "file",
          size: String(file.size || 0),
        });
        if (file.type) {
          params.set("mime_type", file.type);
        }
        const xhr = new XMLHttpRequest();
        xhr.open("POST", `/internal/attachments/upload?${params.toString()}`);
        xhr.setRequestHeader("x-gwt-upload-token", token);
        // Keep failure bodies too: storage errors are plain-text responses.
        xhr.responseType = "text";
        xhr.upload.onprogress = (event) => {
          onProgress?.({
            loaded: event.loaded || 0,
            total: event.lengthComputable ? event.total : file.size || 0,
          });
        };
        xhr.onload = () => {
          if (xhr.status < 200 || xhr.status >= 300) {
            const detail = typeof xhr.response === "string" ? xhr.response : xhr.responseText;
            reject(new Error(`HTTP ${xhr.status}: ${detail || xhr.statusText || "attachment upload failed"}`));
            return;
          }
          try {
            resolve(typeof xhr.response === "object" && xhr.response
              ? xhr.response : JSON.parse(xhr.responseText || xhr.response || "{}"));
          } catch (error) {
            reject(new Error(`Invalid attachment upload response (HTTP ${xhr.status}): ${error.message}`));
          }
        };
        xhr.onerror = () => reject(new Error("attachment upload network error (HTTP 0)"));
        xhr.onabort = () => reject(new Error("upload aborted"));
        signal?.addEventListener("abort", () => xhr.abort(), { once: true });
        xhr.send(file);
      })
      .catch(reject);
  });
}
