# Quickdrop

**Private, instant file transfer between your devices.**

Quickdrop moves files directly between any two devices — in any direction — over
your own local network. Every transfer is end-to-end encrypted. There's no
account, no cloud, no upload limits, and nothing ever leaves your network.

- **Windows ↔ Android ↔ Windows ↔ Android** — send in any direction
- **End-to-end encrypted** with AES on every byte
- **Direct peer-to-peer** — no servers, no relays, no cloud
- **Resumable** — interrupted transfers pick up where they left off
- **No size limits** — bounded only by your hardware and link speed

---

## Download

Grab the latest build from the **[Releases page](https://github.com/typicallhavok/quickdrop/releases/latest)**
or use the direct links below.

| Platform | File | Requirements |
|----------|------|--------------|
| Windows (installer) | [`quickdrop_0.1.0_x64-setup.exe`](https://github.com/typicallhavok/quickdrop/releases/latest/download/quickdrop_0.1.0_x64-setup.exe) | Windows 10 / 11, 64-bit |
| Windows (MSI) | [`quickdrop_0.1.0_x64_en-US.msi`](https://github.com/typicallhavok/quickdrop/releases/latest/download/quickdrop_0.1.0_x64_en-US.msi) | Windows 10 / 11, 64-bit |
| Android (APK) | [`quickdrop.apk`](https://github.com/typicallhavok/quickdrop/releases/latest/download/quickdrop.apk) | Android 8.0+ (sideload) |

The Windows installer (`.exe`) is recommended for most users. The Android app is
sideloaded — it isn't on the Play Store, so you may need to allow installs from
your browser the first time.

A simple landing page for the project lives in [`landing/`](landing/).

---

## Features

- **End-to-end encryption.** Devices verify each other's identity, and you
  choose which ones to trust. Trusted devices skip the prompt on future sends.
- **Direct, no cloud.** Files go straight from one device to the other over your
  local network. Nothing is uploaded, stored, or seen by anyone else.
- **Resume interrupted transfers.** Lost connection or closed the app? Continue
  exactly where you left off instead of restarting the whole file.
- **Clipboard sharing.** Push the text on your clipboard to another device in one
  tap — links, codes, and snippets land instantly.
- **Drag, drop, done.** Drop files onto the window or share straight from
  Android. Multiple files queue up and send together.
- **Automatic discovery.** Nearby devices appear on their own — no IP addresses,
  no pairing codes, no setup to remember.
- **Built for speed.** Tuned socket buffers and large streaming chunks keep the
  connection saturated, so transfers run at the speed of your hardware rather
  than your upload plan.

---

## How it works

1. **Discover.** Open Quickdrop on two devices on the same network and they find
   each other automatically over UDP.
2. **Verify & trust.** Confirm the device once; trusted devices send instantly
   thereafter.
3. **Transfer.** Pick files (or share from anywhere) and they arrive encrypted,
   at full speed, over a direct TCP connection.

Under the hood, Quickdrop uses a custom binary protocol:

- **Discovery** over UDP on port **55433**.
- **Transfer** over TCP on port **55432**.
- An authenticated handshake establishes a shared key, then control messages are
  exchanged as **AES-GCM** frames and the file body streams as a raw **AES-CTR**
  cipher stream.
- Resume is negotiated once per transfer: the receiver decides the offset to
  continue a partial (`.unconfirmed`) file from and tells the sender, who streams
  exactly the remaining bytes.

---

## Project structure

```
share/                    # this repository — desktop app + core engine
├── backend/              # core P2P engine (Rust library)
│   └── src/
│       ├── protocol.rs   # wire protocol, ports, framing
│       ├── crypto.rs     # AES-GCM / AES-CTR encryption
│       ├── handshake.rs  # authenticated key exchange
│       ├── identity.rs   # device identity & trust
│       ├── udp.rs        # device discovery
│       ├── ble.rs        # Bluetooth LE discovery
│       ├── session.rs    # per-connection session state machine
│       ├── transfer.rs   # send/receive file logic
│       └── state.rs      # shared app state
├── frontend/             # desktop GUI (Tauri 2 + SvelteKit + Rust)
│   └── src-tauri/        # Tauri shell wrapping the backend engine
├── landing/              # static marketing / download site (HTML/CSS/JS)
└── README.md
```

The **Android app** lives in a separate repository (`sharedroid/`), written in
Kotlin. It speaks the exact same wire protocol, so any change to discovery or the
transfer protocol must be mirrored on both sides.

---

## Building from source

### Prerequisites

- **Rust** (edition 2024) — <https://rustup.rs>
- **Node.js** 18+ and **npm** — for the desktop frontend
- Tauri 2 platform deps — see <https://tauri.app/start/prerequisites/>
- **Android Studio** / Gradle — only for the Android app

### Core engine (backend)

```sh
cd backend
cargo build --release      # build the library
cargo check                # fast type-check
```

### Desktop app (frontend)

```sh
cd frontend
npm install
npm run tauri dev          # run in development
npm run tauri build        # produce installers (NSIS .exe + .msi)
```

Built installers land in `frontend/src-tauri/target/release/bundle/`
(`nsis/` for the `.exe`, `msi/` for the `.msi`).

### Android app

Build the signed release APK from the `sharedroid` project in Android Studio
(or `./gradlew assembleRelease`). The output APK is what gets published to the
Releases page.

### Landing page

Pure static HTML/CSS/JS — no build step. See [`landing/README.md`](landing/README.md)
for how to serve it and how the download buttons are wired.

---

## Releases

Releases are published on GitHub with the Windows and Android binaries attached:

- **All releases:** <https://github.com/typicallhavok/quickdrop/releases>
- **Latest:** <https://github.com/typicallhavok/quickdrop/releases/latest>

The `releases/latest/download/<file>` links above always resolve to the newest
release, so they're safe to share and embed.

---

## Privacy & security

Quickdrop is designed to keep your data on your own network:

- No account, no telemetry, no cloud storage.
- Files travel directly between devices and are encrypted end to end.
- Devices establish trust explicitly — you approve who can send to you.

---

## Contributing

Quickdrop is open source. Issues and pull requests are welcome. When changing the
discovery or transfer protocol, remember it must stay compatible across **both**
the desktop and Android apps.
