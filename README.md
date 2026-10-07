# agentplug-modlens

Vision bridge for gm's agentplug host, after [liustack/modlens](https://github.com/liustack/modlens). It gives a text-only model structured evidence for an image: a multimodal API reads the pixels and the plugin returns summary, OCR, layout regions, entities and relations, visual notes and an uncertainty list. A `wasm32-wasip1` plugin; the host does the I/O.

## Dispatch

Spool verb `modlens`, body `{"verb": "...", ...}`:

| verb | body | answer |
| --- | --- | --- |
| `read_image` (aliases `read`, `analyze`, `vision`) | one of `path`, `url`, `base64` (+`mime`); optional `mode`, `prompt`, `provider`, `pin`, `model`, `timeoutMs` | `{ok, provider, mode, untrusted_content, result, meta}` |
| `doctor` | `{}` | configured chain, masked keys, missing fields |
| `capabilities` | `{}` | verbs, modes, providers |

`mode` tunes the prompt: `describe` (default), `ocr`, `ui`, `chart`, `diagram`, `error`. `result` follows the modlens v2 contract (`summary`, `ocr`, `layout`, `semantics`, `visual`, `uncertainty`) and is validated before it is returned; a broken result fails over to the next provider. `meta.attempts` records every provider tried.

## Engines

Chain order: `gemini-api`, `openai` (any OpenAI-compatible chat-completions endpoint with image input), `anthropic`. `provider` states a preference (the chain still backs it up); `pin: true` runs that one alone.

Settings come from `.gm/modlens.json` when it names the provider, otherwise from the environment:

| provider | env | defaults |
| --- | --- | --- |
| `gemini-api` | `GEMINI_API_KEY` or `GOOGLE_API_KEY`, `GEMINI_BASE_URL`, `GEMINI_MODEL` | `gemini-3.6-flash` |
| `openai` | `OPENAI_API_KEY`, `OPENAI_BASE_URL`, `OPENAI_MODEL` (model required) | `https://api.openai.com/v1` |
| `anthropic` | `ANTHROPIC_API_KEY`, `ANTHROPIC_BASE_URL`, `ANTHROPIC_MODEL` | `claude-haiku-4-5-20251001` |

```json
{ "provider": "gemini-api",
  "providers": { "openai": { "apiKey": "k1,k2", "baseUrl": "https://dashscope.aliyuncs.com/compatible-mode/v1", "model": "qwen3-vl-plus", "structuredOutput": false, "extraBody": {} } } }
```

`apiKey` takes a comma-separated list; the next key is tried after a 401/403/429/432/433 or a quota message, other failures move to the next provider. Keep `.gm/modlens.json` out of git.

## Safety

- Image text is untrusted: the prompt says so and every answer carries `untrusted_content: true`.
- `url` input is downloaded by the host (http/https only, no userinfo, private, loopback and link-local hosts refused, 20 MB cap, type checked by magic bytes). Redirect targets are not re-checked by the host.
- Keys are masked by `doctor` and scrubbed from error text.

## Host surface

Needs `host_fs_read_base64` and `host_fetch` with `responseEncoding:"base64"` (agentplug host), plus `host_env_get`, `host_fs_read`, `host_now_ms`. Callable from gm through `host_plugin_call` (capability allowlist `gm -> modlens`).

## Build

`cargo build --release --target wasm32-wasip1 --lib` produces `agentplug_modlens.wasm`; CI publishes it as `modlens.wasm` to `AnEntrypoint/agentplug-modlens-bin`.
