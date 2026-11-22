# Contributing

Use synthetic fixtures and mock model responses in tests. Keep credentials, runtime databases, and private documents out of version control. Enforce document permissions on the server for search, citations, previews, and downloads.

Run `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1` from the project root before committing. It checks formatting, lint, tests, and both frontend builds. Run `cargo build --locked` from `server/` to check the server binary.
