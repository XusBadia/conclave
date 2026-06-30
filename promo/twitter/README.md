# Conclave MD — promo images (X / Twitter)

On-brand promotional graphics rendered from HTML using the product's own
design tokens (Bugatti-inspired monochrome palette, JetBrains Mono wordmark,
the cyan-node "C" mark). All in-stream cards are **16:9 @2x (3200×1800)**;
the profile banner is **3:1 (3000×1000)**. X downscales them cleanly.

| File | Size | Use | Suggested copy |
|------|------|-----|----------------|
| `01-hero.png` | 3200×1800 | Launch / pinned tweet — app mockup mid-deliberation | *A virtual clinical committee, on your machine. Local-first clinical decision support that deliberates over **your** protocols — and never phones home. Free & open source.* |
| `02-deliberation.png` | 3200×1800 | The 4-phase loop (brief → draft → critique → verdict) | *Most AI gives you its first guess. Conclave MD puts it on trial: it drafts, critiques itself, then revises before it commits.* |
| `03-privacy.png` | 3200×1800 | Privacy / local-first thread | *Patient data never leaves your machine. No telemetry, de-identification before any prompt, secrets in the OS keychain — enforced in the architecture, not promised in a policy.* |
| `04-providers.png` | 3200×1800 | "Bring your own engine" | *Your model, your keys — or fully offline. Anthropic, OpenAI, OpenRouter, your existing Claude/ChatGPT plan, Ollama or Apple Intelligence.* |
| `05-grounded.png` | 3200×1800 | Citations / auditability | *No black box. Every verdict cites the exact document and page in your own corpus, and surfaces the red flags a single-shot answer skips.* |
| `header.png` | 3000×1000 | X profile banner | — |

> Reminder for any copy: Conclave MD is **not a medical device** — keep the
> disclaimer in the thread.

## Regenerate

Edit the `src/*.html` files (they share `src/style.css`) and run:

```sh
cd promo/twitter/src && python3 build.py        # all
python3 build.py 01-hero.html                   # one page
```

Renders via headless Google Chrome at `--force-device-scale-factor=2`.
Web fonts (JetBrains Mono / Inter) load from Google Fonts at render time, so
the build needs network. The app mockup lives in `src/_app.html` and is
injected into the `__APP__` placeholder.
