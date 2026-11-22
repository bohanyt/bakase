# Bakase

Document chat with citations, Excel, Word and PowerPoint generation, and an Edge side panel that can attach the current page. Document search and downloads respect the signed-in user's access to each source.

Rust and Axum serve the API, SQLite state, Tantivy search, and the React/Vite frontend. Azure OpenAI provides chat and optional document OCR. Optional ONNX embedding inference runs locally.

## Run on Windows

Install Rust 1.89 or newer and Node.js 22 or newer. From the project root, run:

```powershell
cd web
npm ci
npm run build
cd ../server
Copy-Item config.example.toml config.toml
# Set your Azure OpenAI endpoint and deployment in config.toml.
cargo run -- set-azure-key
cargo run -- create-admin admin@example.com "Demo Admin"
cargo run
```

Open http://127.0.0.1:8100. Set `auth.email_domain` for your organization and configure `[[sources]]` in `server/config.toml`. Run `cargo run -- ingest` from `server/` to index your sources.

For the Edge extension, set the app origin in `extension/panel.js`, run `npm run build:extension` from `web/`, and load `extension/` as an unpacked extension. Copy its generated ID from `edge://extensions` into `auth.extension_ids` and restart the server.

## Optional ONNX models

- Text search supports a compatible model of your choice, such as [Granite Embedding 311M Multilingual R2](https://huggingface.co/ibm-granite/granite-embedding-311m-multilingual-r2) (Apache-2.0). Provide `model.onnx`, `tokenizer.json`, any external ONNX weights, and `app-model.toml` with `name`, `pooling`, `dim`, and `normalize`. Set `[makna].model` and run `cargo run -- embed`.
- Image search supports [EmbeddingGemma 2 ONNX](https://huggingface.co/onnx-community/embeddinggemma-2-ONNX) (Apache-2.0). Provide the FP32 image encoder, backbone and weights, tokenizer, and processor settings. Set `[gambar].model` and run `cargo run -- embed-gambar` after indexing the documents.

Set `ORT_DYLIB_PATH` to the local ONNX Runtime library. Obtain the models separately from their publishers.

## License

The code is released under the MIT License in `LICENSE`. The mascot artwork in `web/public/brand/` and `extension/icons/` is copyright bohan, all rights reserved, and is not covered by the MIT License.
