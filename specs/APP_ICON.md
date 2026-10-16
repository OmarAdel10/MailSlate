# MailSlate — App Icon & Brand Marks Specification

| Field | Value |
| --- | --- |
| Version | 1.0 |
| Date | 2026-10-16 |
| Status | Approved for implementation |
| Art status | **Pending** — master artwork to be provided by the project owner (see §1) |
| Related | [PRD](./PRD.md) · [ARCHITECTURE](./ARCHITECTURE.md) · [DESIGN](./DESIGN.md) · [USER_FLOW](./USER_FLOW.md) · [DEVELOPMENT_ENVIRONMENT](./DEVELOPMENT_ENVIRONMENT.md) |

This document defines the MailSlate brand mark theme (the *brief the artwork must satisfy*) and
the complete implementation pipeline that ships it on Linux, Windows, and macOS. The final
artwork does not exist yet — this spec makes the art requirements explicit and defines exactly
what happens the day it lands (§7).

---

## 1. Purpose & status

- **Purpose.** Single source of truth for: (a) the visual brief of the app icon, tray glyph,
  and notification marks; (b) the generation pipeline from master art to per-platform assets;
  (c) wiring into builds, installers, and in-app surfaces.
- **Pipeline status.** Defined here, implemented when art arrives. Until then, dev builds use
  the Flutter default icon — this is acceptable on feature branches and **blocks the M5 ship
  milestone** only (PRD §8 exit criteria).
- **Scope.** Brand mark only. In-UI iconography (navigation, actions) is covered by DESIGN.md §8
  (Material Symbols Rounded) and is unrelated to this file.

---

## 2. Design brief — the icon theme

The mark must read as *calm, monochrome, and precise* — the same design language as the app
chrome (DESIGN.md §1: monochrome action layer, hairlines, no decorative noise).

### 2.1 Metaphor — acceptable directions

Either direction is acceptable; the project owner (or their designer) chooses. Only one ships.

| # | Direction | Description |
| --- | --- | --- |
| A | **Slate envelope** | Abstract, flat envelope silhouette whose flap folds like a slate edge — two or three straight folds, no detail inside the fold. Reads as 'mail' instantly at 16px. |
| B | **Monogram M / folded slate** | A constructed 'M' formed from two chevron-like folds — reads as a monogram at large sizes and as a mountain/slate silhouette at small sizes. |

Both must survive the 16px test (§2.4). No other directions without updating this file.

### 2.2 Palette (from DESIGN.md §3)

| Context | Background | Mark |
| --- | --- | --- |
| Light app tile (default) | `#ffffff` or transparent | `#111111` |
| Dark variant (optional deliverable) | `#101010` | `#fafafa` |
| Tray / notification glyph | transparent | pure black + alpha (macOS template style) |

**Never** on the mark: accent `#3b82f6`, badge pastels, semantic colors, gradients, glass, 3D,
Bevels, drop shadows beyond a subtle 1px hairline, or any text.

### 2.3 Shape & geometry

- **App tile (all OSes):** full-bleed rounded square — corner radius ratio **22.5%** of the side
  (macOS squircle approximation; also used for Windows and Linux for one consistent mark).
- **Safe area:** the glyph occupies the central **80%** of the tile (10% padding per side).
  Nothing outside the safe area except the tile background fill.
- **Master canvas:** 1024×1024.

### 2.4 Legibility rules (hard requirements)

- Glyph must be identifiable at **16×16** rendered size — verify before handoff (§7 step 1).
- Minimum stroke/fold width at 16px render: **1.5px**. Nothing thinner.
- Single silhouette — no strokes-in-strokes, no interior detail that dies below 32px.
- Filled mark only (no outline-only variant), so it holds contrast on any background.

### 2.5 Do / Don't

| Do | Don't |
| --- | --- |
| One flat color mark per variant | Multi-color, gradients, or transparency gradients |
| Generous safe-area padding | Edge-to-edge detail |
| Straight, deliberate folds | Rounded organic blobs |
| Silhouette-first design | Fine typography or ligature detail |
| Monochrome per §2.2 | Accent colors from the UI palette |

---

## 3. Source art deliverables (checklist when art arrives)

All masters are committed under `assets/branding/icon/source/`. Accepted formats: SVG
(preferred, rasterized in-pipeline) or 1024×1024 PNG with alpha (8-bit RGBA).

| File | Purpose | Notes |
| --- | --- | --- |
| `mailslate-icon-master.svg` *or* `.png` | Light app tile, 1024×1024 | Rounded-square tile per §2.3, ink mark on canvas |
| `mailslate-icon-dark.svg` *or* `.png` | Dark tile variant, 1024×1024 | `#fafafa` mark on `#101010`; used for Linux dark theme + optional |
| `mailslate-glyph-mono.svg` | Tray + notification glyph | Pure black + alpha, optimized for 16–24px; source for macOS template images |

Also committed: `assets/branding/icon/source/README.md` — one paragraph describing regeneration
(`tool/generate_icons.sh`) and the art status pointer back to this spec.

---

## 4. Platform output matrix (generated, committed)

Everything below is produced by the pipeline (§5) and **committed to the repository** so builds
never depend on running the script. Sizes are exact.

### 4.1 App icon — Linux

| Artifact | Sizes | Location | Consumer |
| --- | --- | --- | --- |
| PNG set | 16, 24, 32, 48, 64, 128, 256, 512 | `linux/runner/resources/icons/hicolor/{N}x{N}/apps/mailslate.png` | .deb/.rpm/AppImage packaging installs to `/usr/share/icons/hicolor/…`; `.desktop` uses `Icon=mailslate` |
| Window icon | 256 | same set | `window_manager` `setIcon` at runtime |

### 4.2 App icon — Windows

| Artifact | Sizes | Location | Consumer |
| --- | --- | --- | --- |
| ICO | 16, 24, 32, 48, 64, 128, 256 | `windows/runner/resources/app_icon.ico` | EXE resource (referenced by `windows/runner/Runner.rc`), taskbar, Alt-Tab, installer |
| ICO format | — | — | 32-bit alpha, PNG-compressed entries (ICO spec allows PNG payloads for ≥256) |

### 4.3 App icon — macOS

| Artifact | Entries | Location | Consumer |
| --- | --- | --- | --- |
| AppIcon set | 16@1x, 16@2x, 32@1x, 32@2x, 128@1x, 128@2x, 256@1x, 256@2x, 512@1x, 512@2x | `macos/Runner/Assets.xcassets/AppIcon.appiconset/` + `Contents.json` | Dock, Cmd-Tab, Finder, .dmg |
| Daemon agent bundle | same set | `macos/` daemon agent target (LSUIElement, ADR A14) | Notification attribution when the GUI is closed |

### 4.4 Tray & notification marks

| Platform | Artifact | Sizes | Location |
| --- | --- | --- | --- |
| Linux | PNG glyph | 22 + 44 (@2x) | `linux/runner/resources/tray/tray.png`, `tray@2x.png` |
| Windows | ICO glyph | 16, 32 | `windows/runner/resources/tray.ico` |
| macOS | Template PNG | `TrayIconTemplate.png` (16) + `TrayIconTemplate@2x.png` (32) | `macos/Runner/Resources/` — Template suffix gives automatic light/dark inversion |

Notifications: Linux notification servers take the app icon automatically (app name
`MailSlate`); Windows toasts show the EXE icon; macOS notifications show the **daemon bundle
icon** when posted while the GUI is closed — that is why the daemon carries its own copy in
§4.3. No separate notification art is required in v1.

---

## 5. Generation pipeline

### 5.1 Script — `tool/generate_icons.sh`

- **Inputs:** masters from §3 only. No other sources.
- **Outputs:** every artifact in §4 (idempotent — safe to re-run; overwrites generated files).
- **Rasterization:** masters rasterized to 1024 PNG with `magick -filter Lanczos`, then
  downscaled per size (always downscale from 1024, never chain-downscale).
- **Tooling dependencies** (dev machines only; outputs are committed):
  - ImageMagick ≥ 7 (`magick`) — PNG set, ICO composition.
  - `icnsutil` (Python package) for `.icns` composition on Linux/Windows dev boxes; macOS may
    use native `iconutil` instead (script prefers `iconutil` when present).
  - bash. Windows developers: run via WSL, or rely on committed outputs (script not required
    on Windows).
- **Verification:** the script ends by printing a table of every generated file with its size
  and byte count; non-zero exit on any missing output.

### 5.2 Decision — no `flutter_launcher_icons`

Desktop-only pipeline, exact size control, and fewer pub dependencies outweigh the convenience
of `flutter_launcher_icons`. If the team later prefers it, it may replace the script for
Windows/macOS assets only — the Linux hicolor tree still requires the script.

### 5.3 When the pipeline runs

- Once, when masters land (§7).
- Again only when masters change. Generated outputs are committed; no CI step runs the script.
- CD release builds (later pipeline work) assert the presence of required outputs and fail the
  release if the icon set is missing — the M5 guard.

---

## 6. In-app usage map

| Surface | Mark used | Notes |
| --- | --- | --- |
| Taskbar / Dock / window switcher | OS assets from §4 | Platform-managed |
| Window icon (Linux) | 256 PNG | `window_manager` `setIcon` at startup |
| About dialog | Master tile, 64px | Next to wordmark `MailSlate` (title-md 18/600, Inter) |
| Sidebar | **None** — text wordmark only | Calm chrome (DESIGN.md §1); no logo clutter in the 3-pane shell |
| Empty states / onboarding | **None** — type-led | DESIGN.md §7 empty-state pattern; hero imagery is not part of v1 |
| Tray | Glyph from §4.4 | M5 stretch (tray itself optional, USER_FLOW §8) |
| Notifications | Per §4.4 | No custom art |

---

## 7. Art-arrival checklist (definition of done)

When the project owner delivers the masters, this is the closing sequence. It is the entire
scope of the eventual icon work item — nothing more.

1. **Validate masters** — safe area (§2.3), 16px legibility test (§2.4), alpha correctness,
   palette compliance (§2.2).
2. **Run** `tool/generate_icons.sh` — all outputs generated, verification table green.
3. **Review per-OS builds** — open Linux, Windows, and macOS builds and check: window/taskbar/
   dock icon, Alt-Tab / Cmd-Tab, tray glyph, one test notification per OS, .desktop entry,
   installer icon.
4. **Update this file** — change `Art status` to `Landed`, add the dated filenames delivered.
5. **Commit** via the emoji-commit skill — masters, generated assets, spec status update in
   one coherent commit (`✨ feat(branding): …`).

Until step 4, M5 exit criteria are not met (PRD §8).

---

## 8. Cross-links

- [PRD](./PRD.md) — M5 milestone exit criteria reference this checklist.
- [DESIGN](./DESIGN.md) — §8 in-UI iconography (Material Symbols); §1 monochrome discipline.
- [ARCHITECTURE](./ARCHITECTURE.md) — macOS daemon agent bundle (ADR A14) that carries the dock/
  notification icon.
- [USER_FLOW](./USER_FLOW.md) — §8 tray behavior; §10 About dialog.
- [DEVELOPMENT_ENVIRONMENT](./DEVELOPMENT_ENVIRONMENT.md) — ImageMagick / icnsutil dev
  prerequisites listed for the pipeline.
