# EasyTier Desktop UI

Vite + React + TypeScript frontend for the EasyTier desktop shell. The layout is
an operational desktop interface with Overview, Private Network, Peers, Logs,
and Settings views.

## Run

```bash
npm install
npm run dev
```

The Vite development server uses `http://127.0.0.1:1420`.

```bash
npm test
npm run build
```

## Native boundary

`src/lib/bridge.ts` defines the typed, package-free bridge expected from the
desktop host at `window.__TAURI__.core`. The eventual Tauri integration can map
its invokes and events to that interface, or replace this adapter in one place.

The browser preview starts explicitly offline. It does not simulate a successful
core connection, and it stores only visual preferences in `localStorage`.
Profile secrets stay in memory for preview editing and must be handled by the
native secure store in the production bridge.
