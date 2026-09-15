# Fighters Guild Launcher

A small desktop app that installs and keeps the Fighters Guild Minecraft modpack
in sync on top of an official Minecraft installation. 

Built with [Tauri](https://tauri.app) (Rust backend, a small HTML/CSS/JS UI
styled to match the [Fighters Guild Minecraft page](https://fightersguild.playit.quest/minecraft)).

## What it does

1. Finds your existing `.minecraft` folder and confirms vanilla `1.20.1` has
   been launched at least once (that's what downloads Java and the base game
   files — this app doesn't reimplement that).
2. Runs NeoForge's own installer in `--installClient` mode, so the launcher
   profile format is always whatever NeoForge actually expects, not something
   hand-built here.
3. Downloads a manifest published by the Fighters Guild Portal and syncs mods,
   configs, and other pack files to match it (adds new/changed files, verifies
   each one's hash, removes files no longer in the pack).
4. Creates a desktop shortcut to the real Minecraft Launcher.

Every future run re-checks all of the above, so re-running it is how players
update the pack.

## Requirements to build

- Node.js 18+
- Rust (stable, MSVC toolchain on Windows) — see [rustup.rs](https://rustup.rs)
- On Windows: the "Desktop development with C++" workload from Visual Studio
  Build Tools (Tauri's Windows build needs the MSVC linker)

## Development

```
npm install
npm run dev
```

## Building a release

```
npm run build
```

Produces an NSIS installer under `src-tauri/target/release/bundle/nsis/`.

Signing an update-capable release additionally needs `TAURI_SIGNING_PRIVATE_KEY`
(path to the private key) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` set in the
environment. The keypair isn't in this repo — ask whoever's maintaining
releases for it.

## License

AGPL-3.0-or-later. See [LICENSE](LICENSE).
