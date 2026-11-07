# README tour video

`record.js` drives a demo account through the desktop, the phone and the
wearable face from one 1920×1080 "stage" page (the app runs in an iframe:
full screen, phone-shaped, watch-shaped; captions and title cards are HTML on
the stage) and saves CDP screencast frames with their timestamps.
`build.js` turns them into `docs/screenshots/liteaura-tour.mp4` (H.264, 30 fps)
and the README's animated WebP, squeezing each "AI at work" span to 2.5 s.

How it was run (2026-11-07):

1. A second server on port 18091 with its own data in `/tmp/liteaura-demo`
   (`DATABASE_URL=sqlite:///tmp/liteaura-demo/shiny.db`, `PLUGINS_DIR` holding
   word, calc, calendar, files, calculator, impress and pdf, the speech
   sidecars off) — never the server in use.
2. A demo account (`Alex`) registered through `/api/auth/register`, its token
   and traveler JSON in `/tmp/liteaura-demo/token` / `traveler.json`, the
   plugins activated and `ai.ollama_model` set to `gemma4:31b-cloud`.
3. `node record.js`, then `node build.js` (Playwright and ffmpeg; the
   Playwright path at the top of `record.js` is this machine's).

Recording only: the stage strips `X-Frame-Options` / `frame-ancestors` from
the app's HTML document so it can be framed; the server is unchanged. The
wearable's spoken requests are played as a demo (voice and tool events), not
spoken to a model.
