# Kitchen Table

**A home for your small software.**

You built a trip planner for your family in five minutes. Why does sharing it take an afternoon, a cloud account, and making it public to the whole internet?

Kitchen Table turns any folder on your computer into a live web app that the people you choose can open on their phones. No hosting, no deploys, no accounts for your viewers, ever. Drop a folder in, get a local URL and a QR code, decide who can see it. If you use an AI agent, it gets simpler still: ask for an app, and hand your wife a working link thirty seconds later.

> Named for the kitchen-table business: something small, personal, and family-run. This is the same idea for software.

## Status

**Pre-beta, built in the open.** macOS first (in active development), Windows and Linux next, headless Linux (your Raspberry Pi in the cupboard) on the roadmap. Follow progress in [checklist-desktop.md](./checklist-desktop.md). Star the repo if you want this to exist; it genuinely helps.

## How it works

1. **Drop a folder.** Anything in your `~/KitchenTable` workspace becomes an app: HTML, PDFs, a little tool an agent built you. An optional `app.json` names it and sets its visibility.
2. **It's live on your network.** Each app gets `appname.local` with HTTPS and a QR code. Serving happens from your machine; nothing leaves it.
3. **Share on your terms.** Four levels per app: Private, Household (anyone on a network you've marked as home, and it auto-pauses the moment you join an unfamiliar one), Invited (a magic link you can expire, pin to a device, or revoke), and Public. New devices need your explicit approval, like pairing Bluetooth, and every visit lands in a per-app access log.
4. **Apps can remember.** A built-in storage API (`kt.storage()`) gives every app its own little database, shared or per-viewer, so checklists and trackers work with zero backend code.
5. **Agents are first-class.** A built-in MCP server lets Claude Code or any MCP client create apps, deploy files, set sharing, and mint invite links. The whole flow, from "build us a packing list" to a link in the family chat, happens in one conversation.

```text
You:    build us a packing list app for the Lisbon trip and share it with Priya
Agent:  Created "Packing list", set to Invited, here's the link:
        https://packing-list.local/i/kJ82... (QR in your Kitchen Table window)
```

No code executes on your machine except the daemon itself: apps are static files plus storage, and the only place their JavaScript runs is the viewer's own browser.

## What about when my laptop is asleep?

Then your links sleep too; that's the honest deal with serving from your own machine. Two answers: run the (planned) headless daemon on an always-on box you own, or use the optional paid relay, which gives apps stable public URLs and keeps a snapshot alive while your machine is off. The relay is a hosted service and its code is not in this repo; the privacy guarantees that matter are enforced by the open code that is. Invited and private apps relayed in Strict mode are end to end encrypted: the relay routes ciphertext it cannot read, which you can verify in the daemon source rather than take on trust.

## Install

Beta builds will ship on the releases page as a signed, notarized DMG. Until then, build from source:

```bash
git clone https://github.com/kitchen-table/kitchentable && cd kitchentable
cargo run -p kt-daemon        # the daemon, headless
cargo run -p kt-cli -- list   # talk to it
pnpm tauri dev                # the full desktop app (from shell/)
```

Requires stable Rust, Node 20+, and pnpm. See [CLAUDE.md](./CLAUDE.md) for the full command reference (it's written for AI agents working on the codebase, and it works just as well for humans).

## Docs

- [product.md](./docs/product.md): what this is and deliberately isn't
- [architecture.md](./docs/architecture.md): daemon, socket API, auth flows, security model
- [onboarding.md](./docs/onboarding.md): install and permission flows per OS
- [examples/](./examples): the Welcome app and the trip planner that started all this

## How it compares

ngrok and Tailscale Serve expose things for people who already speak dev tools; Kitchen Table is for sharing with people who don't, with approval prompts and revocable links instead of tailnets and tunnels. Vercel and friends are superb at publishing software to everyone; this is for software that was never meant for everyone. And unlike a cloud drive, what you share here is alive: apps with state, not files in a folder.

## Contributing

The work plan is [checklist-desktop.md](./checklist-desktop.md): one PR per checklist item, tests where marked, conventions in [CLAUDE.md](./CLAUDE.md). Issues tagged `good-first-item` are genuinely scoped for a first contribution. Security reports: see [SECURITY.md](./SECURITY.md), please don't open public issues for vulnerabilities.

## Licence

Dual-licensed under MIT or Apache-2.0, at your option. The desktop app, daemon, CLI, and protocol types in this repo are and will remain open source.
