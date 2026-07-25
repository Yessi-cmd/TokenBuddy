# TokenBuddy

TokenBuddy is a local-first desktop observability tool for AI coding token usage.
The product and engineering requirements live in
[`AI_Coding_Token_Observatory_PROJECT_SPEC.md`](AI_Coding_Token_Observatory_PROJECT_SPEC.md).

## Prerequisites

- Node.js 24+
- pnpm 11+
- Rust 1.93+
- Platform prerequisites for [Tauri 2](https://v2.tauri.app/start/prerequisites/)

## Development

```sh
pnpm install
pnpm dev
```

## Verification

```sh
pnpm format:check
pnpm lint
pnpm test
pnpm build:web
pnpm check:rust
```
