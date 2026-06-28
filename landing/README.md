# Quickdrop — landing page

A single static page (no build step) showcasing Quickdrop for Windows and Android.

```
landing/
├── index.html      # landing page (markup + content)
├── download.html   # downloads page — pick installer / format / platform
├── styles.css      # styles (one accent color, no gradients)
├── script.js       # mobile nav, footer year, subtle reveal-on-scroll
├── downloads/
│   ├── quickdrop_0.1.0_x64-setup.exe   # Windows NSIS installer
│   ├── quickdrop_0.1.0_x64_en-US.msi   # Windows MSI
│   └── android/
│       └── quickdrop.apk               # Android app
└── README.md
```

## Run it

Just open `index.html` in a browser. To serve it locally:

```sh
# from the landing/ folder
python -m http.server 8080
# then visit http://localhost:8080
```

## Downloads

Two ways to download are wired up, side by side:

1. **Local files** (work right now). The current builds are already copied into
   `downloads/`. The hero buttons and the download page link straight to them.
   - Windows installer (`.exe` / `.msi`) — produced by `npm run tauri build`
     (NSIS + MSI) in `frontend/`.
   - Android `.apk` — from the Gradle release build in `sharedroid/`.
   - To publish a new version, rebuild, copy the new artifacts into `downloads/`
     (and `downloads/android/`), and bump the filenames/version on the pages.

2. **GitHub Releases mirror** (works once you publish a release). The
   "via GitHub Releases" links on `download.html` use the direct-download form:

   ```
   https://github.com/typicallhavok/quickdrop/releases/latest/download/<asset-name>
   ```

   This downloads the asset straight from the latest release — the user never
   has to browse GitHub. Just create a release and upload assets with the same
   filenames the page references. To host *only* on GitHub (keep the binaries
   out of this repo), point every button's `href` at that URL form and delete
   the local files in `downloads/`.

The full chooser (all formats + both mirrors) lives on `download.html`; the
landing page's hero/footer buttons are quick links to the recommended builds.

## Deploy

It's fully static, so any host works: GitHub Pages, Netlify, Cloudflare Pages,
or any web server — upload the folder as-is.
