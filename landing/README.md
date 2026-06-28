# Quickdrop — landing page

A single static page (no build step) showcasing Quickdrop for Windows and Android.

```
landing/
├── index.html      # markup + content
├── styles.css      # styles (one accent color, no gradients)
├── script.js       # mobile nav, footer year, subtle reveal-on-scroll
├── downloads/      # put your release binaries here (see below)
└── README.md
```

## Run it

Just open `index.html` in a browser. To serve it locally:

```sh
# from the landing/ folder
python -m http.server 8080
# then visit http://localhost:8080
```

## Wire up the download buttons

The Windows and Android buttons point to:

- `downloads/Quickdrop-Setup.exe`
- `downloads/Quickdrop.apk`

Drop your built artifacts into a `downloads/` folder with those names, **or**
edit the `href`s in `index.html` (two spots: hero + the Download section) to
point at your real release URLs (e.g. a GitHub Releases page).

- Windows installer is produced by `npm run tauri build` (NSIS `.exe` + `.msi`).
- Android `.apk` comes from the Gradle build.

## Deploy

It's fully static, so any host works: GitHub Pages, Netlify, Cloudflare Pages,
or any web server — upload the folder as-is.
